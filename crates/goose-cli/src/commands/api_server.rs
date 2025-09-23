use crate::database::DatabaseManager;
use anyhow::{Context, Result};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Json,
    },
    routing::{get, post},
    Router,
};
use futures::{stream, StreamExt};
use goose::agents::{Agent, AgentEvent};
use goose::conversation::message::Message as GooseMessage;
use goose::conversation::Conversation;
use goose::providers::base::Provider;
use rmcp::model::Tool;
use serde::{Deserialize, Serialize};
use std::{convert::Infallible, net::SocketAddr, sync::Arc, time::SystemTime};
use tower_http::cors::{Any, CorsLayer};
use tracing::{debug, error, info, warn};

// Updated AppState with direct provider access
#[derive(Clone)]
struct AppState {
    agent: Arc<Agent>,
    provider: Arc<dyn Provider>, // Direct provider access for token streaming
    db: Arc<DatabaseManager>,
}

#[derive(Serialize)]
struct SessionCreateResponse {
    session_id: String,
}

#[derive(Serialize)]
struct SessionResponse {
    session_id: String,
    created_at: String,
    updated_at: String,
    message_count: usize,
}

#[derive(Serialize)]
struct SessionListResponse {
    sessions: Vec<SessionResponse>,
    total: usize,
}

#[derive(Deserialize)]
struct MessageRequest {
    message: String,
}

#[derive(Serialize)]
struct StreamEvent {
    #[serde(rename = "type")]
    event_type: String,
    content: serde_json::Value,
}

#[derive(Serialize)]
struct TokenStreamEvent {
    #[serde(rename = "type")]
    event_type: String,
    content: String,
    accumulated: String,
    timestamp: u64,
    session_id: String,
}

pub async fn handle_api_server(
    port: u16,
    host: String,
    database_url: Option<String>,
) -> Result<()> {
    info!("🚀 Starting Goose API Server initialization...");

    // Log environment variables for debugging
    debug!("🔍 Environment Variables:");
    debug!(
        "GOOSE_AUTH_BYPASS: {:?}",
        std::env::var("GOOSE_AUTH_BYPASS")
    );
    debug!("MONGODB_URL: {:?}", std::env::var("MONGODB_URL"));
    debug!("MONGODB_DATABASE: {:?}", std::env::var("MONGODB_DATABASE"));
    debug!("RUST_LOG: {:?}", std::env::var("RUST_LOG"));
    debug!("Database URL arg: {:?}", database_url);

    // Load Goose configuration
    info!("🔧 Loading Goose configuration...");
    let config = goose::config::Config::global();

    let provider_name: String = config
        .get_param("GOOSE_PROVIDER")
        .context("No provider configured. Run 'goose configure' first")?;

    let model: String = config
        .get_param("GOOSE_MODEL")
        .context("No model configured. Run 'goose configure' first")?;

    info!("🤖 Using provider: {} with model: {}", provider_name, model);

    // Create provider directly for token streaming
    info!("🔌 Creating direct provider access...");
    let model_config =
        goose::model::ModelConfig::new(&model).context("Failed to create model config")?;
    let provider = goose::providers::create(&provider_name, model_config.clone())
        .context("Failed to create provider")?;

    info!(
        "🌊 Provider streaming support: {}",
        provider.supports_streaming()
    );

    // Create and configure agent (existing pattern)
    info!("🔌 Setting up Goose agent...");
    let agent = Agent::new();
    let agent_provider = goose::providers::create(&provider_name, model_config)
        .context("Failed to create agent provider")?;
    agent
        .update_provider(agent_provider)
        .await
        .context("Failed to update agent provider")?;

    // Load extensions
    debug!("🧩 Loading extensions...");
    let extensions = goose::config::ExtensionConfigManager::get_all()
        .context("Failed to load extension configuration")?;
    let mut loaded_extensions = 0;

    for ext_config in extensions {
        if ext_config.enabled {
            match agent.add_extension(ext_config.config.clone()).await {
                Ok(_) => {
                    loaded_extensions += 1;
                    debug!("✅ Loaded extension: {}", ext_config.config.name());
                }
                Err(e) => {
                    warn!(
                        "⚠️ Failed to load extension {}: {}",
                        ext_config.config.name(),
                        e
                    );
                }
            }
        }
    }
    info!("🧩 Loaded {} extensions", loaded_extensions);

    // Initialize MongoDB connection (REQUIRED)
    let db_url = database_url
        .or_else(|| std::env::var("MONGODB_URL").ok())
        .unwrap_or_else(|| {
            error!("❌ MongoDB URL is required!");
            error!("Provide via --database-url or MONGODB_URL environment variable");
            error!("Example: --database-url mongodb://localhost:27017");
            error!("This API server requires MongoDB - no local storage fallback");
            std::process::exit(1);
        });

    let db_name =
        std::env::var("MONGODB_DATABASE").unwrap_or_else(|_| "goose_sessions".to_string());

    info!("📊 Initializing MongoDB connection...");
    let db_manager = DatabaseManager::new(&db_url, &db_name)
        .await
        .context("Failed to initialize MongoDB connection")?;

    let app_state = AppState {
        agent: Arc::new(agent),
        provider: provider, // Direct provider access for token streaming
        db: Arc::new(db_manager),
    };

    // Build router with both streaming endpoints
    info!("🌍 Setting up API routes...");
    let app = Router::new()
        .route("/api/v1/health", get(health_check))
        .route("/api/v1/sessions", post(create_session).get(list_sessions))
        .route(
            "/api/v1/sessions/{id}",
            get(get_session).delete(delete_session),
        )
        .route("/api/v1/sessions/{id}/messages", post(send_message)) // Existing agent-level streaming
        .route(
            "/api/v1/sessions/{id}/stream",
            post(stream_direct_from_provider),
        ) // NEW: Provider-level token streaming
        .route("/api/v1/sessions/{id}/export", get(export_session))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        )
        .with_state(app_state);

    let addr: SocketAddr = format!("{}:{}", host, port)
        .parse()
        .context("Failed to parse server address")?;

    info!("✨ Goose API Server Ready!");
    info!("🌐 Listening on: http://{}", addr);
    info!("📊 Database: {} ({})", db_name, db_url);
    info!("🔧 Storage: MongoDB-only (no local fallback)");
    info!("OPENAI_API_KEY: {}", {
        std::env::var("OPENAI_API_KEY").unwrap_or_else(|_| "unset".to_string())
    });
    info!("BLACKBOX_API_KEY: {}", {
        std::env::var("BLACKBOX_API_KEY").unwrap_or_else(|_| "unset".to_string())
    });
    info!("OPENAI_HOST: {}", {
        std::env::var("OPENAI_HOST").unwrap_or_else(|_| "unset".to_string())
    });
    info!("OPENAI_BASE_PATH: {}", {
        std::env::var("OPENAI_BASE_PATH").unwrap_or_else(|_| "unset".to_string())
    });
    info!("GOOSE_MODEL: {}", {
        std::env::var("GOOSE_MODEL").unwrap_or_else(|_| "unset".to_string())
    });
    info!("📡 Endpoints: 8 REST API endpoints available");
    info!("🌊 Streaming modes: Agent-level (/messages) + Provider-level (/stream)");

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .context("Failed to bind to address")?;
    axum::serve(listener, app)
        .await
        .context("Server failed during operation")?;

    Ok(())
}

// Health check endpoint with streaming mode info
// Health check endpoint with streaming mode info and environment variables
async fn health_check(State(state): State<AppState>) -> impl IntoResponse {
    debug!("❤️ Health check requested");

    let timestamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    // Test MongoDB connection
    let database_connected = state.db.health_check().await;

    // Collect environment variables for debugging
    let env_vars = serde_json::json!({
        "GOOSE_PROVIDER": std::env::var("GOOSE_PROVIDER").unwrap_or("NOT_SET".to_string()),
        "OPENAI_HOST": std::env::var("OPENAI_HOST").unwrap_or("NOT_SET".to_string()),
        "OPENAI_BASE_PATH": std::env::var("OPENAI_BASE_PATH").unwrap_or("NOT_SET".to_string()),
        "GOOSE_LEAD_MODEL": std::env::var("GOOSE_LEAD_MODEL").unwrap_or("NOT_SET".to_string()),
        "GOOSE_AUTH_BYPASS": std::env::var("GOOSE_AUTH_BYPASS").unwrap_or("NOT_SET".to_string()),
        "MONGODB_URL": std::env::var("MONGODB_URL").unwrap_or("NOT_SET".to_string()),
        "MONGODB_DATABASE": std::env::var("MONGODB_DATABASE").unwrap_or("NOT_SET".to_string()),
        "RUST_LOG": std::env::var("RUST_LOG").unwrap_or("NOT_SET".to_string())
    });

    let response = serde_json::json!({
        "status": if database_connected { "ok" } else { "error" },
        "timestamp": timestamp.to_string(),
        "version": env!("CARGO_PKG_VERSION"),
        "database_connected": database_connected,
        "storage_type": "mongodb",
        "mongodb_url": state.db.connection_url.clone(),
        "mongodb_database": state.db.db_name.clone(),
        "streaming_modes": ["agent-events", "provider-tokens"],
        "environment_variables": env_vars,
        "provider_streaming_support": state.provider.supports_streaming()
    });

    let status_code = if database_connected {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    if database_connected {
        debug!("✅ Health check: OK");
    } else {
        error!("❌ Health check: MongoDB connection failed");
    }

    // Log environment variables to console for debugging
    info!("🔍 Environment Variables Debug:");
    info!("GOOSE_PROVIDER: {:?}", std::env::var("GOOSE_PROVIDER"));
    info!("OPENAI_HOST: {:?}", std::env::var("OPENAI_HOST"));
    info!("OPENAI_BASE_PATH: {:?}", std::env::var("OPENAI_BASE_PATH"));
    info!("GOOSE_LEAD_MODEL: {:?}", std::env::var("GOOSE_LEAD_MODEL"));
    info!(
        "GOOSE_AUTH_BYPASS: {:?}",
        std::env::var("GOOSE_AUTH_BYPASS")
    );
    info!("MONGODB_URL: {:?}", std::env::var("MONGODB_URL"));

    (status_code, Json(response))
}

// Create session endpoint
async fn create_session(State(state): State<AppState>) -> impl IntoResponse {
    info!("➕ Creating new session...");

    match state.db.create_session().await {
        Ok(session_id) => {
            info!("✅ Session created successfully: {}", session_id);
            (
                StatusCode::CREATED,
                Json(SessionCreateResponse { session_id }),
            )
        }
        Err(e) => {
            error!("❌ Failed to create session in MongoDB: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(SessionCreateResponse {
                    session_id: "error".to_string(),
                }),
            )
        }
    }
}

// List all sessions
async fn list_sessions(State(state): State<AppState>) -> impl IntoResponse {
    info!("📋 Listing sessions from MongoDB...");

    match state.db.list_sessions().await {
        Ok(sessions) => {
            debug!(
                "🔍 Found {} sessions, getting message counts...",
                sessions.len()
            );

            let mut session_responses = Vec::new();
            for session in sessions {
                let message_count = state
                    .db
                    .get_message_count(&session.session_id)
                    .await
                    .unwrap_or_else(|e| {
                        warn!(
                            "⚠️ Failed to get message count for {}: {}",
                            session.session_id, e
                        );
                        0
                    });

                session_responses.push(SessionResponse {
                    session_id: session.session_id,
                    created_at: session.created_at.clone(),
                    updated_at: format!("{:?}", session.updated_at),
                    message_count,
                });
            }

            let total = session_responses.len();
            info!("✅ Listed {} sessions from MongoDB", total);
            Json(SessionListResponse {
                sessions: session_responses,
                total,
            })
        }
        Err(e) => {
            error!("❌ Failed to list sessions from MongoDB: {}", e);
            Json(SessionListResponse {
                sessions: vec![],
                total: 0,
            })
        }
    }
}

// Get session details
async fn get_session(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    info!("🔍 Getting session details: {}", session_id);

    match state.db.get_session(&session_id).await {
        Ok(Some(session)) => {
            debug!("✅ Found session in MongoDB: {}", session_id);

            let message_count = state
                .db
                .get_message_count(&session_id)
                .await
                .unwrap_or_else(|e| {
                    warn!("⚠️ Failed to get message count: {}", e);
                    0
                });

            let response = SessionResponse {
                session_id: session.session_id,
                created_at: session.created_at.clone(),
                updated_at: format!("{:?}", session.updated_at),
                message_count,
            };

            (StatusCode::OK, Json(response))
        }
        Ok(None) => {
            debug!("❌ Session not found in MongoDB: {}", session_id);
            (
                StatusCode::NOT_FOUND,
                Json(SessionResponse {
                    session_id: "not_found".to_string(),
                    created_at: "".to_string(),
                    updated_at: "".to_string(),
                    message_count: 0,
                }),
            )
        }
        Err(e) => {
            error!(
                "❌ Failed to query session {} from MongoDB: {}",
                session_id, e
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(SessionResponse {
                    session_id: "error".to_string(),
                    created_at: "".to_string(),
                    updated_at: "".to_string(),
                    message_count: 0,
                }),
            )
        }
    }
}

// Delete session
async fn delete_session(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    info!("🗑️ Deleting session: {}", session_id);

    match state.db.delete_session(&session_id).await {
        Ok(true) => {
            info!("✅ Successfully deleted session: {}", session_id);
            StatusCode::NO_CONTENT
        }
        Ok(false) => {
            debug!("🔍 Session not found for deletion: {}", session_id);
            StatusCode::NOT_FOUND
        }
        Err(e) => {
            error!(
                "❌ Failed to delete session {} from MongoDB: {}",
                session_id, e
            );
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

// EXISTING: Agent-level streaming (unchanged)
async fn send_message(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
    Json(request): Json<MessageRequest>,
) -> impl IntoResponse {
    info!(
        "💬 [Agent-Level] Processing message for session {}",
        session_id
    );
    debug!("📝 Message content: {}", request.message);

    // Verify session exists in MongoDB
    match state.db.get_session(&session_id).await {
        Ok(Some(_)) => {
            debug!("✅ Session found in MongoDB: {}", session_id);
        }
        Ok(None) => {
            warn!("❌ Session not found in MongoDB: {}", session_id);
            let error_data = serde_json::to_string(&StreamEvent {
                event_type: "error".to_string(),
                content: serde_json::json!({
                    "message": "Session not found in MongoDB",
                    "session_id": session_id.clone()
                }),
            })
            .unwrap();

            return Sse::new(stream::once(async move {
                Ok::<_, Infallible>(Event::default().data(error_data))
            }))
            .keep_alive(KeepAlive::default())
            .into_response();
        }
        Err(e) => {
            let error_msg = e.to_string();
            error!(
                "❌ Failed to check session {} in MongoDB: {}",
                session_id, error_msg
            );
            let error_data = serde_json::to_string(&StreamEvent {
                event_type: "error".to_string(),
                content: serde_json::json!({
                    "message": "MongoDB connection error",
                    "error": error_msg
                }),
            })
            .unwrap();

            return Sse::new(stream::once(async move {
                Ok::<_, Infallible>(Event::default().data(error_data))
            }))
            .keep_alive(KeepAlive::default())
            .into_response();
        }
    }

    // Save user message to MongoDB
    debug!("💾 Saving user message to MongoDB...");
    let user_message = GooseMessage::user().with_text(&request.message);
    if let Err(e) = state.db.add_message(&session_id, &user_message).await {
        let error_msg = e.to_string();
        error!("❌ Failed to save user message to MongoDB: {}", error_msg);
        let error_data = serde_json::to_string(&StreamEvent {
            event_type: "error".to_string(),
            content: serde_json::json!({
                "message": "Failed to save message to MongoDB",
                "error": error_msg
            }),
        })
        .unwrap();

        return Sse::new(stream::once(async move {
            Ok::<_, Infallible>(Event::default().data(error_data))
        }))
        .keep_alive(KeepAlive::default())
        .into_response();
    }
    debug!("✅ User message saved to MongoDB");

    // Load conversation history from MongoDB
    debug!("📖 Loading conversation history from MongoDB...");
    let conversation = match state.db.get_conversation(&session_id).await {
        Ok(conv) => {
            info!(
                "✅ Loaded conversation with {} messages from MongoDB",
                conv.messages().len()
            );
            conv
        }
        Err(e) => {
            let error_msg = e.to_string();
            error!("❌ Failed to load conversation from MongoDB: {}", error_msg);
            let error_data = serde_json::to_string(&StreamEvent {
                event_type: "error".to_string(),
                content: serde_json::json!({
                    "message": "Failed to load conversation from MongoDB",
                    "error": error_msg
                }),
            })
            .unwrap();

            return Sse::new(stream::once(async move {
                Ok::<_, Infallible>(Event::default().data(error_data))
            }))
            .keep_alive(KeepAlive::default())
            .into_response();
        }
    };

    // Create agent stream with MongoDB persistence
    info!("🤖 [Agent-Level] Starting agent streaming response...");
    let agent_stream = create_agent_stream(
        state.agent.clone(),
        state.db.clone(),
        session_id.clone(),
        conversation,
    );

    Sse::new(agent_stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

// NEW: Provider-level token streaming endpoint
async fn stream_direct_from_provider(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
    Json(request): Json<MessageRequest>,
) -> impl IntoResponse {
    info!(
        "🌊 [Provider-Level] Starting token streaming for session {}",
        session_id
    );
    debug!("📝 Message content: {}", request.message);

    // Verify session exists in MongoDB
    match state.db.get_session(&session_id).await {
        Ok(Some(_)) => {
            debug!("✅ Session found in MongoDB: {}", session_id);
        }
        Ok(None) => {
            warn!("❌ Session not found in MongoDB: {}", session_id);
            let error_data = serde_json::to_string(&TokenStreamEvent {
                event_type: "error".to_string(),
                content: "Session not found in MongoDB".to_string(),
                accumulated: "".to_string(),
                timestamp: SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap()
                    .as_secs(),
                session_id: session_id.clone(),
            })
            .unwrap();

            return Sse::new(stream::once(async move {
                Ok::<_, Infallible>(Event::default().data(error_data))
            }))
            .keep_alive(KeepAlive::default())
            .into_response();
        }
        Err(e) => {
            let error_msg = e.to_string();
            error!(
                "❌ Failed to check session {} in MongoDB: {}",
                session_id, error_msg
            );
            let error_data = serde_json::to_string(&TokenStreamEvent {
                event_type: "error".to_string(),
                content: format!("MongoDB connection error: {}", error_msg),
                accumulated: "".to_string(),
                timestamp: SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap()
                    .as_secs(),
                session_id: session_id.clone(),
            })
            .unwrap();

            return Sse::new(stream::once(async move {
                Ok::<_, Infallible>(Event::default().data(error_data))
            }))
            .keep_alive(KeepAlive::default())
            .into_response();
        }
    }

    // Save user message to MongoDB
    debug!("💾 Saving user message to MongoDB...");
    let user_message = GooseMessage::user().with_text(&request.message);
    if let Err(e) = state.db.add_message(&session_id, &user_message).await {
        let error_msg = e.to_string();
        error!("❌ Failed to save user message to MongoDB: {}", error_msg);
        let error_data = serde_json::to_string(&TokenStreamEvent {
            event_type: "error".to_string(),
            content: format!("Failed to save message to MongoDB: {}", error_msg),
            accumulated: "".to_string(),
            timestamp: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            session_id: session_id.clone(),
        })
        .unwrap();

        return Sse::new(stream::once(async move {
            Ok::<_, Infallible>(Event::default().data(error_data))
        }))
        .keep_alive(KeepAlive::default())
        .into_response();
    }
    debug!("✅ User message saved to MongoDB");

    // Load conversation history from MongoDB
    debug!("📖 Loading conversation history from MongoDB...");
    let conversation = match state.db.get_conversation(&session_id).await {
        Ok(conv) => {
            info!(
                "✅ Loaded conversation with {} messages from MongoDB",
                conv.messages().len()
            );
            conv
        }
        Err(e) => {
            let error_msg = e.to_string();
            error!("❌ Failed to load conversation from MongoDB: {}", error_msg);
            let error_data = serde_json::to_string(&TokenStreamEvent {
                event_type: "error".to_string(),
                content: format!("Failed to load conversation from MongoDB: {}", error_msg),
                accumulated: "".to_string(),
                timestamp: SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap()
                    .as_secs(),
                session_id: session_id.clone(),
            })
            .unwrap();

            return Sse::new(stream::once(async move {
                Ok::<_, Infallible>(Event::default().data(error_data))
            }))
            .keep_alive(KeepAlive::default())
            .into_response();
        }
    };

    // Create provider-level token stream
    info!("🌊 [Provider-Level] Starting direct token streaming...");
    let token_stream = create_provider_token_stream(
        state.provider.clone(),
        state.db.clone(),
        session_id.clone(),
        conversation,
    );

    Sse::new(token_stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

// Export session as markdown from MongoDB
async fn export_session(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    info!("📤 Exporting session: {}", session_id);

    match state.db.get_conversation(&session_id).await {
        Ok(conversation) => {
            debug!(
                "✅ Loaded conversation for export: {} messages",
                conversation.messages().len()
            );

            let mut markdown = format!("# Session Export: {}\n\n", session_id);
            markdown.push_str(&format!("**Database:** {}\n", state.db.db_name));
            markdown.push_str(&format!(
                "**Exported:** {}\n\n",
                chrono::Utc::now().to_rfc3339()
            ));

            for (i, message) in conversation.messages().iter().enumerate() {
                let role = format!("{:?}", message.role);
                let content = message.as_concat_text();
                markdown.push_str(&format!(
                    "## Message {} - {}\n\n{}\n\n---\n\n",
                    i + 1,
                    role,
                    content
                ));
            }

            info!(
                "✅ Exported session {} ({} messages)",
                session_id,
                conversation.messages().len()
            );
            (StatusCode::OK, markdown)
        }
        Err(e) => {
            error!(
                "❌ Failed to export session {} from MongoDB: {}",
                session_id, e
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to export session from MongoDB".to_string(),
            )
        }
    }
}

// EXISTING: Agent-level event stream (unchanged)
fn create_agent_stream(
    agent: Arc<Agent>,
    db: Arc<DatabaseManager>,
    session_id: String,
    conversation: Conversation,
) -> impl futures::Stream<Item = Result<Event, Infallible>> {
    async_stream::stream! {
        info!("🤖 [Agent-Level] Starting agent reply for session: {}", session_id);
        debug!("📖 Using conversation with {} messages", conversation.messages().len());

        match agent.reply(conversation.clone(), None, None).await {
            Ok(mut event_stream) => {
                info!("✅ Agent reply stream started");
                let mut assistant_content = String::new();

                while let Some(result) = event_stream.next().await {
                    match result {
                        Ok(AgentEvent::Message(msg)) => {
                            let content = msg.as_concat_text();
                            debug!("💬 Agent message: {:?} - {} chars", msg.role, content.len());

                            let event_data = StreamEvent {
                                event_type: "message".to_string(),
                                content: serde_json::json!({
                                    "role": format!("{:?}", msg.role),
                                    "content": content,
                                    "timestamp": SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs(),
                                    "session_id": session_id
                                }),
                            };

                            // Collect assistant content for saving to MongoDB
                            let role_str = format!("{:?}", msg.role);
                            if role_str == "Assistant" {
                                assistant_content.push_str(&content);
                            }

                            if let Ok(data) = serde_json::to_string(&event_data) {
                                yield Ok(Event::default().data(data));
                            }
                        },
                        Ok(AgentEvent::McpNotification(notif)) => {
                            debug!("🔔 MCP notification received");

                            let event_data = StreamEvent {
                                event_type: "tool_notification".to_string(),
                                content: serde_json::to_value(&notif).unwrap_or_default(),
                            };

                            if let Ok(data) = serde_json::to_string(&event_data) {
                                yield Ok(Event::default().data(data));
                            }
                        },
                        Ok(AgentEvent::ModelChange { .. }) => {
                            debug!("🔄 Model change event");
                            // Model change events do not need special handling for now
                        },
                        Ok(AgentEvent::HistoryReplaced(new_messages)) => {
                            info!("🔄 Agent replaced conversation history with {} messages", new_messages.len());

                            let event_data = StreamEvent {
                                event_type: "history_update".to_string(),
                                content: serde_json::json!({
                                    "message_count": new_messages.len(),
                                    "session_id": session_id
                                }),
                            };

                            if let Ok(data) = serde_json::to_string(&event_data) {
                                yield Ok(Event::default().data(data));
                            }

                            // Update MongoDB with new conversation history
                            debug!("💾 Updating MongoDB with new conversation history...");
                            let new_conversation = Conversation::new_unvalidated(new_messages);
                            if let Err(e) = db.update_conversation(&session_id, &new_conversation).await {
                                error!("❌ Failed to update conversation in MongoDB: {}", e);
                            } else {
                                debug!("✅ Updated conversation in MongoDB");
                            }
                        },
                        Err(e) => {
                            error!("❌ Agent stream error: {}", e);

                            let event_data = StreamEvent {
                                event_type: "error".to_string(),
                                content: serde_json::json!({
                                    "message": "Agent processing error",
                                    "error": e.to_string(),
                                    "session_id": session_id
                                }),
                            };

                            if let Ok(data) = serde_json::to_string(&event_data) {
                                yield Ok(Event::default().data(data));
                            }
                            break;
                        }
                    }
                }

                // Save final assistant response to MongoDB
                if !assistant_content.is_empty() {
                    debug!("💾 Saving assistant response to MongoDB ({} chars)", assistant_content.len());
                    let assistant_message = GooseMessage::assistant().with_text(&assistant_content);
                    if let Err(e) = db.add_message(&session_id, &assistant_message).await {
                        error!("❌ Failed to save assistant message to MongoDB: {}", e);
                    } else {
                        debug!("✅ Assistant response saved to MongoDB");
                    }
                }

                // Send completion event
                info!("✅ [Agent-Level] Streaming completed for session: {}", session_id);
                let event_data = StreamEvent {
                    event_type: "complete".to_string(),
                    content: serde_json::json!({
                        "message": "Response complete",
                        "session_id": session_id,
                        "storage": "mongodb"
                    }),
                };

                if let Ok(data) = serde_json::to_string(&event_data) {
                    yield Ok(Event::default().data(data));
                }
            },
            Err(e) => {
                error!("❌ Failed to start agent reply: {}", e);

                let event_data = StreamEvent {
                    event_type: "error".to_string(),
                    content: serde_json::json!({
                        "message": "Failed to start agent processing",
                        "error": e.to_string(),
                        "session_id": session_id
                    }),
                };

                if let Ok(data) = serde_json::to_string(&event_data) {
                    yield Ok(Event::default().data(data));
                }
            }
        }
    }
}

// NEW: Provider-level token streaming function
fn create_provider_token_stream(
    provider: Arc<dyn Provider>,
    db: Arc<DatabaseManager>,
    session_id: String,
    conversation: Conversation,
) -> impl futures::Stream<Item = Result<Event, Infallible>> {
    async_stream::stream! {
        info!("🌊 [Provider-Level] Starting direct provider streaming for session: {}", session_id);
        debug!("📖 Using conversation with {} messages", conversation.messages().len());

        // Prepare provider call parameters
        let messages = conversation.messages();
        let system_prompt = "You are a helpful AI assistant."; // TODO: Get proper system prompt
        let tools: Vec<Tool> = vec![]; // TODO: Get tools from agent if needed

        let mut accumulated_content = String::new();
        let mut token_count = 0;

        match provider.stream(system_prompt, messages, &tools).await {
            Ok(mut message_stream) => {
                info!("✅ [Provider-Level] MessageStream started");

                while let Some(result) = message_stream.next().await {
                    match result {
                        Ok((Some(message), usage)) => {
                            // This is a token chunk from the provider!
                            let chunk_content = message.as_concat_text();
                            accumulated_content.push_str(&chunk_content);
                            token_count += 1;

                            debug!("🎫 Token {}: '{}' (accumulated: {} chars)",
                                   token_count, chunk_content, accumulated_content.len());

                            // Stream the token immediately
                            let event_data = TokenStreamEvent {
                                event_type: "token".to_string(),
                                content: chunk_content,
                                accumulated: accumulated_content.clone(),
                                timestamp: SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs(),
                                session_id: session_id.clone(),
                            };

                            if let Ok(data) = serde_json::to_string(&event_data) {
                                yield Ok(Event::default().data(data));
                            }

                            // Emit usage info if available
                            if let Some(usage_info) = usage {
                                let usage_event = TokenStreamEvent {
                                    event_type: "usage".to_string(),
                                    content: format!("Tokens: {}", usage_info.usage.total_tokens.unwrap_or(0)),
                                    accumulated: accumulated_content.clone(),
                                    timestamp: SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs(),
                                    session_id: session_id.clone(),
                                };

                                if let Ok(data) = serde_json::to_string(&usage_event) {
                                    yield Ok(Event::default().data(data));
                                }
                            }
                        },
                        Ok((None, usage)) => {
                            // Usage-only event (no content)
                            if let Some(usage_info) = usage {
                                debug!("📊 Usage info: {:?}", usage_info);
                                let usage_event = TokenStreamEvent {
                                    event_type: "usage_only".to_string(),
                                    content: format!("Final usage: {}", usage_info.usage.total_tokens.unwrap_or(0)),
                                    accumulated: accumulated_content.clone(),
                                    timestamp: SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs(),
                                    session_id: session_id.clone(),
                                };

                                if let Ok(data) = serde_json::to_string(&usage_event) {
                                    yield Ok(Event::default().data(data));
                                }
                            }
                        },
                        Err(e) => {
                            error!("❌ [Provider-Level] Stream error: {}", e);

                            let error_event = TokenStreamEvent {
                                event_type: "error".to_string(),
                                content: format!("Provider streaming error: {}", e),
                                accumulated: accumulated_content.clone(),
                                timestamp: SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs(),
                                session_id: session_id.clone(),
                            };

                            if let Ok(data) = serde_json::to_string(&error_event) {
                                yield Ok(Event::default().data(data));
                            }
                            break;
                        }
                    }
                }

                // Save complete assistant response to MongoDB
                if !accumulated_content.is_empty() {
                    info!("💾 [Provider-Level] Saving accumulated response to MongoDB ({} chars, {} tokens)",
                          accumulated_content.len(), token_count);

                    let assistant_message = GooseMessage::assistant().with_text(&accumulated_content);
                    if let Err(e) = db.add_message(&session_id, &assistant_message).await {
                        error!("❌ Failed to save accumulated message to MongoDB: {}", e);
                    } else {
                        debug!("✅ [Provider-Level] Accumulated response saved to MongoDB");
                    }
                }

                // Send completion event
                info!("✅ [Provider-Level] Token streaming completed for session: {}", session_id);
                let completion_event = TokenStreamEvent {
                    event_type: "complete".to_string(),
                    content: "Token streaming complete".to_string(),
                    accumulated: accumulated_content,
                    timestamp: SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs(),
                    session_id: session_id.clone(),
                };

                if let Ok(data) = serde_json::to_string(&completion_event) {
                    yield Ok(Event::default().data(data));
                }
            },
            Err(e) => {
                error!("❌ [Provider-Level] Failed to start provider streaming: {}", e);

                let error_event = TokenStreamEvent {
                    event_type: "error".to_string(),
                    content: format!("Failed to start provider streaming: {}", e),
                    accumulated: "".to_string(),
                    timestamp: SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs(),
                    session_id: session_id.clone(),
                };

                if let Ok(data) = serde_json::to_string(&error_event) {
                    yield Ok(Event::default().data(data));
                }
            }
        }
    }
}
