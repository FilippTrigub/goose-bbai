use crate::database::DatabaseManager;
use anyhow::{Context, Result};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Json,
    },
    routing::{delete, get, post, put},
    Router,
};
use async_stream;
use futures::{stream, StreamExt};
use goose::agents::{Agent, AgentEvent};
use goose::conversation::message::Message as GooseMessage;
use goose::conversation::Conversation;
use goose::providers::base::Provider;
use goose::config::{ExtensionConfigManager, ExtensionEntry};
use goose::agents::ExtensionConfig;
use rmcp::model::Tool;
use serde::{Deserialize, Serialize};
use std::{convert::Infallible, net::SocketAddr, sync::{Arc, RwLock}, time::{SystemTime}, collections::HashMap};
use tower_http::cors::{Any, CorsLayer};
use tracing::{debug, error, info, warn};

// Agent status structures
#[derive(Debug, Clone, Serialize)]
struct AgentStatus {
    session_id: String,
    endpoint_type: String,
    status: String, // "idle", "processing", "completed", "error"
    start_time: SystemTime,
    last_update: SystemTime,
}

#[derive(Serialize)]
struct AgentStatusResponse {
    overall_status: String,
    active_sessions: usize,
    total_processed: usize,
    uptime_seconds: u64,
    sessions: Vec<AgentStatus>,
}

// Updated AppState with direct provider access and agent status tracking
#[derive(Clone)]
struct AppState {
    agent: Arc<Agent>,
    provider: Arc<dyn Provider>, // Direct provider access for token streaming
    db: Arc<DatabaseManager>,
    agent_status: Arc<RwLock<HashMap<String, AgentStatus>>>,
    server_start_time: SystemTime,
    processed_count: Arc<RwLock<usize>>,
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
struct MessageResponse {
    message: String,
    session_id: String,
    assistant_response: String,
    timestamp: u64,
    tool_calls_count: usize,
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

#[derive(Serialize)]
struct SessionMessage {
    role: String,
    content: String,
    timestamp: Option<u64>,
    message_index: usize,
}

#[derive(Serialize)]
struct SessionMessagesResponse {
    session_id: String,
    messages: Vec<SessionMessage>,
    total_count: usize,
}

// Extension management structures
#[derive(Serialize)]
struct ExtensionInfo {
    name: String,
    display_name: Option<String>,
    #[serde(rename = "type")]
    extension_type: String,
    enabled: bool,
    timeout: Option<u64>,
    // For stdio extensions
    cmd: Option<String>,
    args: Option<Vec<String>>,
    // For built-in extensions
    bundled: Option<bool>,
    description: Option<String>,
}

#[derive(Serialize)]
struct ExtensionListResponse {
    extensions: Vec<ExtensionInfo>,
    total_count: usize,
}

#[derive(Deserialize)]
struct ExtensionCreateRequest {
    name: String,
    display_name: Option<String>,
    #[serde(rename = "type")]
    extension_type: String, // "stdio", "builtin", "sse", "streamable_http", "frontend", "inline_python"
    timeout: Option<u64>,
    // For stdio extensions
    cmd: Option<String>,
    args: Option<Vec<String>>,
    // For built-in extensions
    bundled: Option<bool>,
    description: Option<String>,
    // Environment variables
    envs: Option<HashMap<String, String>>,
    env_keys: Option<Vec<String>>,
    // For remote extensions
    uri: Option<String>,
    headers: Option<HashMap<String, String>>,
    // For inline python extensions
    python_code: Option<String>,
}

#[derive(Deserialize)]
struct ExtensionToggleRequest {
    enabled: bool,
}

// Settings management structures
#[derive(Serialize, Clone)]
struct SettingValidation {
    #[serde(skip_serializing_if = "Option::is_none")]
    min: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    allowed_values: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pattern: Option<String>,
}

#[derive(Serialize)]
struct SettingInfo {
    key: String,
    value: Option<serde_json::Value>,
    default_value: Option<serde_json::Value>,
    description: String,
    value_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    validation: Option<SettingValidation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    env_override: Option<String>,
    restart_required: bool,
}

#[derive(Serialize)]
struct SettingsListResponse {
    settings: Vec<SettingInfo>,
    total_count: usize,
}

#[derive(Deserialize)]
struct SettingUpdateRequest {
    value: serde_json::Value,
}

#[derive(Deserialize)]
struct BulkSettingsUpdateRequest {
    settings: HashMap<String, serde_json::Value>,
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

    let server_start_time = SystemTime::now();
    let app_state = AppState {
        agent: Arc::new(agent),
        provider: provider, // Direct provider access for token streaming
        db: Arc::new(db_manager),
        agent_status: Arc::new(RwLock::new(HashMap::new())),
        server_start_time,
        processed_count: Arc::new(RwLock::new(0)),
    };

    // Build router with extension management endpoints
    info!("🌍 Setting up API routes...");
    let app = Router::new()
        .route("/api/v1/health", get(health_check))
        .route("/api/v1/sessions", post(create_session).get(list_sessions))
        .route(
            "/api/v1/sessions/{id}",
            get(get_session).delete(delete_session),
        )
        .route("/api/v1/sessions/{id}/messages", post(send_message).get(get_session_messages)) // Streaming version
        .route("/api/v1/sessions/{id}/send", post(send_message_sync)) // Non-streaming fire-and-forget version
        .route(
            "/api/v1/sessions/{id}/stream",
            post(stream_direct_from_provider),
        ) // NEW: Provider-level token streaming
        .route("/api/v1/sessions/{id}/export", get(export_session))
        // NEW: Extension management endpoints
        .route("/api/v1/extensions", get(list_extensions).post(create_extension))
        .route("/api/v1/extensions/{name}/toggle", put(toggle_extension))
        .route("/api/v1/extensions/{name}", delete(remove_extension))
        // NEW: Settings management endpoints
        .route("/api/v1/settings", get(list_settings).put(update_bulk_settings))
        .route("/api/v1/settings/{key}", get(get_setting).put(update_setting).delete(reset_setting))
        // NEW: Agent status endpoint
        .route("/api/v1/agent/status", get(get_agent_status))
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
    info!("📡 Endpoints: 19 REST API endpoints available");
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

// NEW: Agent status management functions
fn set_agent_status(state: &AppState, session_id: &str, endpoint_type: &str, status: &str) {
    let mut status_map = state.agent_status.write().unwrap();
    let now = SystemTime::now();
    
    let agent_status = AgentStatus {
        session_id: session_id.to_string(),
        endpoint_type: endpoint_type.to_string(),
        status: status.to_string(),
        start_time: if status == "processing" { now } else { 
            status_map.get(session_id).map(|s| s.start_time).unwrap_or(now)
        },
        last_update: now,
    };
    
    status_map.insert(session_id.to_string(), agent_status);
    
    // Increment processed count when completing
    if status == "completed" {
        let mut count = state.processed_count.write().unwrap();
        *count += 1;
    }
    
    debug!("🔄 Agent status updated: {} -> {} ({})", session_id, status, endpoint_type);
}

fn cleanup_old_sessions(state: &AppState) {
    let mut status_map = state.agent_status.write().unwrap();
    let now = SystemTime::now();
    let timeout_duration = std::time::Duration::from_secs(300); // 5 minutes
    
    status_map.retain(|_, status| {
        match now.duration_since(status.last_update) {
            Ok(duration) => {
                let should_keep = duration < timeout_duration;
                if !should_keep {
                    debug!("🧹 Cleaning up old session: {}", status.session_id);
                }
                should_keep
            },
            Err(_) => true, // Keep if we can't determine duration
        }
    });
}

// NEW: Get agent status endpoint
async fn get_agent_status(State(state): State<AppState>) -> impl IntoResponse {
    info!("📊 Getting agent status...");
    
    // Clean up old sessions first
    cleanup_old_sessions(&state);
    
    let status_map = state.agent_status.read().unwrap();
    let processed_count = *state.processed_count.read().unwrap();
    
    // Count active sessions
    let active_sessions = status_map.values()
        .filter(|s| s.status == "processing")
        .count();
    
    // Determine overall status
    let overall_status = if active_sessions > 0 {
        "processing"
    } else {
        "idle"
    };
    
    // Calculate uptime
    let uptime_seconds = SystemTime::now()
        .duration_since(state.server_start_time)
        .unwrap_or_default()
        .as_secs();
    
    // Collect session statuses
    let sessions: Vec<AgentStatus> = status_map.values().cloned().collect();
    
    let response = AgentStatusResponse {
        overall_status: overall_status.to_string(),
        active_sessions,
        total_processed: processed_count,
        uptime_seconds,
        sessions,
    };
    
    info!("✅ Agent status: {} active, {} total processed", active_sessions, processed_count);
    
    (StatusCode::OK, Json(response))
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

// NEW: Settings management endpoints

// Settings registry with metadata
fn get_settings_registry() -> Vec<(&'static str, &'static str, &'static str, Option<SettingValidation>, bool)> {
    // (key, description, value_type, validation, restart_required)
    vec![
        (
            "GOOSE_MODEL",
            "Current LLM model for the agent",
            "string",
            None,
            true,
        ),
        (
            "GOOSE_TEMPERATURE", 
            "Model temperature for response creativity (0.0-2.0)",
            "number",
            Some(SettingValidation {
                min: Some(0.0),
                max: Some(2.0),
                allowed_values: None,
                pattern: None,
            }),
            false,
        ),
        (
            "GOOSE_MODE",
            "Agent behavior mode controlling tool usage and permissions", 
            "string",
            Some(SettingValidation {
                min: None,
                max: None,
                allowed_values: Some(vec!["auto".to_string(), "approve".to_string(), "smart_approve".to_string(), "chat".to_string()]),
                pattern: None,
            }),
            false,
        ),
        (
            "GOOSE_MAX_TURNS",
            "Maximum number of consecutive agent actions without user input",
            "number",
            Some(SettingValidation {
                min: Some(1.0),
                max: None,
                allowed_values: None,
                pattern: None,
            }),
            false,
        ),
        (
            "GOOSE_CLI_MIN_PRIORITY",
            "Minimum priority level for displaying tool output (0.0=all, 0.2=medium, 0.8=high)",
            "number", 
            Some(SettingValidation {
                min: Some(0.0),
                max: Some(1.0),
                allowed_values: None,
                pattern: None,
            }),
            false,
        ),
        (
            "GOOSE_RECIPE_GITHUB_REPO",
            "GitHub repository for Goose recipes (format: owner/repo)",
            "string",
            Some(SettingValidation {
                min: None,
                max: None,
                allowed_values: None,
                pattern: Some(r"^[a-zA-Z0-9_.-]+/[a-zA-Z0-9_.-]+$".to_string()),
            }),
            false,
        ),
        (
            "GOOSE_AUTO_COMPACT_THRESHOLD",
            "Token threshold for automatic context compaction",
            "number",
            Some(SettingValidation {
                min: Some(0.0),
                max: Some(1.0),
                allowed_values: None,
                pattern: None,
            }),
            false,
        ),
    ]
}

// Get default value for a setting
fn get_setting_default(key: &str) -> Option<serde_json::Value> {
    match key {
        "GOOSE_MODE" => Some(serde_json::Value::String("smart_approve".to_string())),
        "GOOSE_MAX_TURNS" => Some(serde_json::Value::Number(serde_json::Number::from(1000))),
        "GOOSE_CLI_MIN_PRIORITY" => Some(serde_json::Value::Number(serde_json::Number::from_f64(0.0).unwrap())),
        "GOOSE_TEMPERATURE" => Some(serde_json::Value::Number(serde_json::Number::from_f64(0.7).unwrap())),
        "GOOSE_AUTO_COMPACT_THRESHOLD" => Some(serde_json::Value::Number(serde_json::Number::from_f64(0.8).unwrap())),
        _ => None,
    }
}

// Validate setting value
fn validate_setting_value(key: &str, value: &serde_json::Value, validation: &Option<SettingValidation>) -> Result<(), String> {
    if let Some(validation_rules) = validation {
        // Type validation
        match key {
            "GOOSE_TEMPERATURE" | "GOOSE_CLI_MIN_PRIORITY" | "GOOSE_AUTO_COMPACT_THRESHOLD" => {
                if !value.is_number() {
                    return Err("Value must be a number".to_string());
                }
                
                let num_val = value.as_f64().ok_or("Invalid number format")?;
                
                if let Some(min) = validation_rules.min {
                    if num_val < min {
                        return Err(format!("Value must be at least {}", min));
                    }
                }
                
                if let Some(max) = validation_rules.max {
                    if num_val > max {
                        return Err(format!("Value must be at most {}", max));
                    }
                }
            },
            "GOOSE_MAX_TURNS" => {
                if !value.is_number() {
                    return Err("Value must be a number".to_string());
                }
                
                let num_val = value.as_u64().ok_or("Must be a positive integer")?;
                
                if num_val < 1 {
                    return Err("Value must be at least 1".to_string());
                }
            },
            _ => {
                if !value.is_string() {
                    return Err("Value must be a string".to_string());
                }
            }
        }
        
        // Allowed values validation
        if let Some(allowed) = &validation_rules.allowed_values {
            let string_val = value.as_str().ok_or("Value must be a string for enum validation")?;
            if !allowed.contains(&string_val.to_string()) {
                return Err(format!("Value must be one of: {}", allowed.join(", ")));
            }
        }
        
        // Pattern validation
        if let Some(pattern) = &validation_rules.pattern {
            let string_val = value.as_str().ok_or("Value must be a string for pattern validation")?;
            let regex = regex::Regex::new(pattern).map_err(|_| "Invalid regex pattern")?;
            if !regex.is_match(string_val) {
                return Err(format!("Value does not match required format: {}", pattern));
            }
        }
    }
    
    Ok(())
}

// List all settings
async fn list_settings() -> impl IntoResponse {
    info!("⚙️ Listing all settings...");
    
    let config = goose::config::Config::global();
    let registry = get_settings_registry();
    let mut settings_info = Vec::new();
    
    for (key, description, value_type, validation, restart_required) in registry {
        // Check for environment variable override
        let env_override = std::env::var(key).ok();
        
        // Get current value from config
        let current_value: Option<serde_json::Value> = if env_override.is_some() {
            // Use env var value
            env_override.as_ref().map(|v| {
                match value_type {
                    "number" => {
                        if let Ok(num) = v.parse::<f64>() {
                            serde_json::Value::Number(serde_json::Number::from_f64(num).unwrap_or(serde_json::Number::from(0)))
                        } else {
                            serde_json::Value::String(v.clone())
                        }
                    },
                    _ => serde_json::Value::String(v.clone())
                }
            })
        } else {
            // Try to get from config file
            match value_type {
                "number" => config.get_param::<f64>(key).ok().map(|v| serde_json::Value::Number(serde_json::Number::from_f64(v).unwrap_or(serde_json::Number::from(0)))),
                _ => config.get_param::<String>(key).ok().map(|v| serde_json::Value::String(v))
            }
        };
        
        let setting_info = SettingInfo {
            key: key.to_string(),
            value: current_value,
            default_value: get_setting_default(key),
            description: description.to_string(),
            value_type: value_type.to_string(),
            validation: validation.clone(),
            env_override: env_override,
            restart_required,
        };
        
        settings_info.push(setting_info);
    }
    
    let total_count = settings_info.len();
    info!("✅ Listed {} settings", total_count);
    
    (
        StatusCode::OK,
        Json(SettingsListResponse {
            settings: settings_info,
            total_count,
        }),
    )
}

// Get specific setting
async fn get_setting(Path(key): Path<String>) -> impl IntoResponse {
    info!("🔍 Getting setting: {}", key);
    
    let registry = get_settings_registry();
    let setting_meta = registry.iter().find(|(k, _, _, _, _)| *k == key);
    
    let (_, description, value_type, validation, restart_required) = match setting_meta {
        Some(meta) => meta,
        None => {
            debug!("❌ Setting not found: {}", key);
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({
                    "error": "Setting not found",
                    "message": format!("Setting {} is not supported", key)
                })),
            );
        }
    };
    
    let config = goose::config::Config::global();
    
    // Check for environment variable override
    let env_override = std::env::var(&key).ok();
    
    // Get current value
    let current_value: Option<serde_json::Value> = if env_override.is_some() {
        // Use env var value
        env_override.as_ref().map(|v| {
            match *value_type {
                "number" => {
                    if let Ok(num) = v.parse::<f64>() {
                        serde_json::Value::Number(serde_json::Number::from_f64(num).unwrap_or(serde_json::Number::from(0)))
                    } else {
                        serde_json::Value::String(v.clone())
                    }
                },
                _ => serde_json::Value::String(v.clone())
            }
        })
    } else {
        // Try to get from config file
        match *value_type {
            "number" => config.get_param::<f64>(&key).ok().map(|v| serde_json::Value::Number(serde_json::Number::from_f64(v).unwrap_or(serde_json::Number::from(0)))),
            _ => config.get_param::<String>(&key).ok().map(|v| serde_json::Value::String(v))
        }
    };
    
    let setting_info = SettingInfo {
        key: key.clone(),
        value: current_value,
        default_value: get_setting_default(&key),
        description: description.to_string(),
        value_type: value_type.to_string(),
        validation: validation.clone(),
        env_override,
        restart_required: *restart_required,
    };
    
    info!("✅ Retrieved setting: {}", key);
    
    (
        StatusCode::OK,
        Json(serde_json::to_value(setting_info).unwrap()),
    )
}

// Update specific setting
async fn update_setting(
    Path(key): Path<String>,
    Json(request): Json<SettingUpdateRequest>,
) -> impl IntoResponse {
    info!("⚙️ Updating setting {}: {:?}", key, request.value);
    
    // Check if setting is supported
    let registry = get_settings_registry();
    let setting_meta = registry.iter().find(|(k, _, _, _, _)| *k == key);
    
    let (_, _, _, validation, restart_required) = match setting_meta {
        Some(meta) => meta,
        None => {
            debug!("❌ Setting not supported: {}", key);
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({
                    "error": "Setting not supported",
                    "message": format!("Setting {} is not configurable via API", key)
                })),
            );
        }
    };
    
    // Check if setting is overridden by environment variable
    if std::env::var(&key).is_ok() {
        debug!("❌ Setting {} is overridden by environment variable", key);
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "Setting overridden",
                "message": format!("Setting {} is overridden by environment variable and cannot be modified", key)
            })),
        );
    }
    
    // Validate the new value
    if let Err(validation_error) = validate_setting_value(&key, &request.value, validation) {
        error!("❌ Validation failed for {}: {}", key, validation_error);
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "Validation failed",
                "message": validation_error
            })),
        );
    }
    
    // Update the setting
    let config = goose::config::Config::global();
    
    match config.set_param(&key, request.value.clone()) {
        Ok(_) => {
            info!("✅ Setting {} updated successfully", key);
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "message": "Setting updated successfully",
                    "key": key,
                    "value": request.value,
                    "restart_required": restart_required
                })),
            )
        }
        Err(e) => {
            error!("❌ Failed to update setting {}: {}", key, e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "Failed to update setting",
                    "message": e.to_string()
                })),
            )
        }
    }
}

// Reset setting to default
async fn reset_setting(Path(key): Path<String>) -> impl IntoResponse {
    info!("🔄 Resetting setting to default: {}", key);
    
    // Check if setting is supported
    let registry = get_settings_registry();
    let setting_meta = registry.iter().find(|(k, _, _, _, _)| *k == key);
    
    if setting_meta.is_none() {
        debug!("❌ Setting not supported: {}", key);
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": "Setting not supported",
                "message": format!("Setting {} is not configurable via API", key)
            })),
        );
    }
    
    // Check if setting is overridden by environment variable
    if std::env::var(&key).is_ok() {
        debug!("❌ Setting {} is overridden by environment variable", key);
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "Setting overridden",
                "message": format!("Setting {} is overridden by environment variable and cannot be reset", key)
            })),
        );
    }
    
    let config = goose::config::Config::global();
    
    // Delete the setting to reset to default
    match config.delete(&key) {
        Ok(_) => {
            let default_value = get_setting_default(&key);
            info!("✅ Setting {} reset to default", key);
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "message": "Setting reset to default successfully",
                    "key": key,
                    "default_value": default_value
                })),
            )
        }
        Err(e) => {
            error!("❌ Failed to reset setting {}: {}", key, e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "Failed to reset setting",
                    "message": e.to_string()
                })),
            )
        }
    }
}

// Bulk update settings
async fn update_bulk_settings(Json(request): Json<BulkSettingsUpdateRequest>) -> impl IntoResponse {
    info!("⚙️ Bulk updating {} settings", request.settings.len());
    
    let registry = get_settings_registry();
    let config = goose::config::Config::global();
    
    let mut results = Vec::new();
    let mut success_count = 0;
    let mut any_restart_required = false;
    
    let total_settings = request.settings.len();
    
    for (key, value) in &request.settings {
        // Check if setting is supported
        let setting_meta = registry.iter().find(|(k, _, _, _, _)| *k == key);
        
        let (_, _, _, validation, restart_required) = match setting_meta {
            Some(meta) => meta,
            None => {
                results.push(serde_json::json!({
                    "key": key,
                    "success": false,
                    "error": format!("Setting {} is not supported", key)
                }));
                continue;
            }
        };
        
        // Check if setting is overridden by environment variable
        if std::env::var(&key).is_ok() {
            results.push(serde_json::json!({
                "key": key,
                "success": false,
                "error": format!("Setting {} is overridden by environment variable", key)
            }));
            continue;
        }
        
        // Validate the value
        if let Err(validation_error) = validate_setting_value(&key, &value, validation) {
            results.push(serde_json::json!({
                "key": key,
                "success": false,
                "error": validation_error
            }));
            continue;
        }
        
        // Update the setting
        match config.set_param(&key, value.clone()) {
            Ok(_) => {
                success_count += 1;
                if *restart_required {
                    any_restart_required = true;
                }
                results.push(serde_json::json!({
                    "key": key,
                    "success": true,
                    "value": value,
                    "restart_required": restart_required
                }));
            }
            Err(e) => {
                results.push(serde_json::json!({
                    "key": key,
                    "success": false,
                    "error": e.to_string()
                }));
            }
        }
    }
    
    info!("✅ Bulk update completed: {}/{} settings updated", success_count, total_settings);
    
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "message": format!("Bulk update completed: {}/{} settings updated", success_count, total_settings),
            "success_count": success_count,
            "total_count": total_settings,
            "restart_required": any_restart_required,
            "results": results
        })),
    )
}

// NEW: Extension management endpoints

// List all extensions
async fn list_extensions() -> impl IntoResponse {
    info!("🧩 Listing all extensions...");

    match ExtensionConfigManager::get_all() {
        Ok(extensions) => {
            let mut extension_infos = Vec::new();
            
            for ext_entry in extensions {
                let ext_config = &ext_entry.config;
                
                let extension_info = ExtensionInfo {
                    name: ext_config.name().clone(),
                    display_name: match ext_config {
                        ExtensionConfig::Builtin { display_name, .. } => display_name.clone(),
                        ExtensionConfig::Stdio { .. } => None,
                        ExtensionConfig::Sse { .. } => None,
                        ExtensionConfig::StreamableHttp { .. } => None,
                        ExtensionConfig::Frontend { .. } => None,
                        ExtensionConfig::InlinePython { .. } => None,
                    },
                    extension_type: match ext_config {
                        ExtensionConfig::Builtin { .. } => "builtin".to_string(),
                        ExtensionConfig::Stdio { .. } => "stdio".to_string(),
                        ExtensionConfig::Sse { .. } => "sse".to_string(),
                        ExtensionConfig::StreamableHttp { .. } => "streamable_http".to_string(),
                        ExtensionConfig::Frontend { .. } => "frontend".to_string(),
                        ExtensionConfig::InlinePython { .. } => "inline_python".to_string(),
                    },
                    enabled: ext_entry.enabled,
                    timeout: match ext_config {
                        ExtensionConfig::Builtin { timeout, .. } => *timeout,
                        ExtensionConfig::Stdio { timeout, .. } => *timeout,
                        ExtensionConfig::Sse { timeout, .. } => *timeout,
                        ExtensionConfig::StreamableHttp { timeout, .. } => *timeout,
                        ExtensionConfig::Frontend { .. } => None, // Frontend doesn't have timeout
                        ExtensionConfig::InlinePython { timeout, .. } => *timeout,
                    },
                    cmd: match ext_config {
                        ExtensionConfig::Stdio { cmd, .. } => Some(cmd.clone()),
                        _ => None,
                    },
                    args: match ext_config {
                        ExtensionConfig::Stdio { args, .. } => Some(args.clone()),
                        _ => None,
                    },
                    bundled: match ext_config {
                        ExtensionConfig::Builtin { bundled, .. } => *bundled,
                        _ => None,
                    },
                    description: match ext_config {
                        ExtensionConfig::Builtin { description, .. } => description.clone(),
                        ExtensionConfig::Stdio { description, .. } => description.clone(),
                        ExtensionConfig::Sse { description, .. } => description.clone(),
                        ExtensionConfig::StreamableHttp { description, .. } => description.clone(),
                        ExtensionConfig::Frontend { instructions, .. } => instructions.clone(),
                        ExtensionConfig::InlinePython { description, .. } => description.clone(),
                    },
                };
                
                extension_infos.push(extension_info);
            }
            
            // Sort extensions alphabetically
            extension_infos.sort_by(|a, b| a.name.cmp(&b.name));
            
            let total_count = extension_infos.len();
            info!("✅ Listed {} extensions", total_count);
            
            (
                StatusCode::OK,
                Json(ExtensionListResponse {
                    extensions: extension_infos,
                    total_count,
                }),
            )
        }
        Err(e) => {
            error!("❌ Failed to list extensions: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ExtensionListResponse {
                    extensions: vec![],
                    total_count: 0,
                }),
            )
        }
    }
}

// Create new extension
async fn create_extension(Json(request): Json<ExtensionCreateRequest>) -> impl IntoResponse {
    info!("➕ Creating extension: {}", request.name);
    
    // Validate required fields based on extension type
    let extension_config = match request.extension_type.as_str() {
        "builtin" => {
            ExtensionConfig::Builtin {
                name: request.name.clone(),
                display_name: request.display_name,
                timeout: request.timeout,
                bundled: request.bundled,
                description: request.description,
                available_tools: Vec::new(),
            }
        }
        "stdio" => {
            let cmd = match request.cmd {
                Some(cmd) => cmd,
                None => {
                    error!("❌ cmd is required for stdio extensions");
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(serde_json::json!({
                            "error": "Missing required field",
                            "message": "cmd is required for stdio extensions"
                        })),
                    );
                }
            };
            let args = request.args.unwrap_or_default();
            
            ExtensionConfig::Stdio {
                name: request.name.clone(),
                cmd,
                args,
                envs: goose::agents::extension::Envs::new(request.envs.unwrap_or_default()),
                env_keys: request.env_keys.unwrap_or_default(),
                description: request.description,
                timeout: request.timeout,
                bundled: request.bundled,
                available_tools: Vec::new(),
            }
        }
        "sse" => {
            let uri = match request.uri {
                Some(uri) => uri,
                None => {
                    error!("❌ uri is required for sse extensions");
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(serde_json::json!({
                            "error": "Missing required field",
                            "message": "uri is required for sse extensions"
                        })),
                    );
                }
            };
            
            ExtensionConfig::Sse {
                name: request.name.clone(),
                uri,
                envs: goose::agents::extension::Envs::new(request.envs.unwrap_or_default()),
                env_keys: request.env_keys.unwrap_or_default(),
                description: request.description,
                timeout: request.timeout,
                bundled: request.bundled,
                available_tools: Vec::new(),
            }
        }
        "streamable_http" => {
            let uri = match request.uri {
                Some(uri) => uri,
                None => {
                    error!("❌ uri is required for streamable_http extensions");
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(serde_json::json!({
                            "error": "Missing required field",
                            "message": "uri is required for streamable_http extensions"
                        })),
                    );
                }
            };
            
            ExtensionConfig::StreamableHttp {
                name: request.name.clone(),
                uri,
                envs: goose::agents::extension::Envs::new(request.envs.unwrap_or_default()),
                env_keys: request.env_keys.unwrap_or_default(),
                headers: request.headers.unwrap_or_default(),
                description: request.description,
                timeout: request.timeout,
                bundled: request.bundled,
                available_tools: Vec::new(),
            }
        }
        "frontend" => {
            ExtensionConfig::Frontend {
                name: request.name.clone(),
                tools: Vec::new(),
                instructions: Some(request.description.unwrap_or_default()),
                available_tools: Vec::new(),
                bundled: request.bundled,
            }
        }
        "inline_python" => {
            let python_code = match request.python_code {
                Some(code) => code,
                None => {
                    error!("❌ python_code is required for inline_python extensions");
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(serde_json::json!({
                            "error": "Missing required field",
                            "message": "python_code is required for inline_python extensions"
                        })),
                    );
                }
            };
            
            ExtensionConfig::InlinePython {
                name: request.name.clone(),
                code: python_code,
                dependencies: Some(Vec::new()),
                available_tools: Vec::new(),
                description: request.description,
                timeout: request.timeout,
            }
        }
        _ => {
            error!("❌ Invalid extension type: {}", request.extension_type);
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "Invalid extension type",
                    "message": format!("Unsupported extension type: {}", request.extension_type)
                })),
            );
        }
    };
    
    let extension_entry = ExtensionEntry {
        enabled: true, // New extensions are enabled by default
        config: extension_config,
    };
    
    match ExtensionConfigManager::set(extension_entry) {
        Ok(_) => {
            info!("✅ Extension {} created successfully", request.name);
            (
                StatusCode::CREATED,
                Json(serde_json::json!({
                    "message": "Extension created successfully",
                    "name": request.name,
                    "enabled": true
                })),
            )
        }
        Err(e) => {
            error!("❌ Failed to create extension {}: {}", request.name, e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "Failed to create extension",
                    "message": e.to_string()
                })),
            )
        }
    }
}

// Toggle extension enabled/disabled
async fn toggle_extension(
    Path(name): Path<String>,
    Json(request): Json<ExtensionToggleRequest>,
) -> impl IntoResponse {
    info!("🔄 Toggling extension {}: enabled={}", name, request.enabled);
    
    let key = goose::config::extensions::name_to_key(&name);
    
    match ExtensionConfigManager::set_enabled(&key, request.enabled) {
        Ok(_) => {
            info!("✅ Extension {} toggled successfully", name);
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "message": "Extension toggled successfully",
                    "name": name,
                    "enabled": request.enabled
                })),
            )
        }
        Err(e) => {
            error!("❌ Failed to toggle extension {}: {}", name, e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "Failed to toggle extension",
                    "message": e.to_string()
                })),
            )
        }
    }
}

// Remove extension (only if disabled)
async fn remove_extension(Path(name): Path<String>) -> impl IntoResponse {
    info!("🗑️ Removing extension: {}", name);
    
    // Check if extension exists and is disabled
    match ExtensionConfigManager::get_all() {
        Ok(extensions) => {
            let extension = extensions.iter().find(|ext| ext.config.name() == name);
            
            match extension {
                Some(ext) if ext.enabled => {
                    debug!("❌ Extension {} is enabled, cannot remove", name);
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(serde_json::json!({
                            "error": "Extension is enabled",
                            "message": "Cannot remove enabled extension. Disable it first."
                        })),
                    );
                }
                Some(_) => {
                    // Extension exists and is disabled, proceed with removal
                }
                None => {
                    debug!("❌ Extension {} not found", name);
                    return (
                        StatusCode::NOT_FOUND,
                        Json(serde_json::json!({
                            "error": "Extension not found",
                            "message": format!("Extension {} does not exist", name)
                        })),
                    );
                }
            }
        }
        Err(e) => {
            error!("❌ Failed to check extensions: {}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "Failed to check extensions",
                    "message": e.to_string()
                })),
            );
        }
    }
    
    let key = goose::config::extensions::name_to_key(&name);
    
    match ExtensionConfigManager::remove(&key) {
        Ok(_) => {
            info!("✅ Extension {} removed successfully", name);
            (
                StatusCode::NO_CONTENT,
                Json(serde_json::json!({
                    "message": "Extension removed successfully",
                    "name": name
                })),
            )
        }
        Err(e) => {
            error!("❌ Failed to remove extension {}: {}", name, e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "Failed to remove extension",
                    "message": e.to_string()
                })),
            )
        }
    }
}

// NEW: Get session messages endpoint
async fn get_session_messages(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    info!("📋 Getting messages for session: {}", session_id);

    // Verify session exists in MongoDB
    match state.db.get_session(&session_id).await {
        Ok(Some(_)) => {
            debug!("✅ Session found in MongoDB: {}", session_id);
        }
        Ok(None) => {
            debug!("❌ Session not found in MongoDB: {}", session_id);
            return (
                StatusCode::NOT_FOUND,
                Json(SessionMessagesResponse {
                    session_id: session_id.clone(),
                    messages: vec![],
                    total_count: 0,
                }),
            );
        }
        Err(e) => {
            error!(
                "❌ Failed to query session {} from MongoDB: {}",
                session_id, e
            );
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(SessionMessagesResponse {
                    session_id: session_id.clone(),
                    messages: vec![],
                    total_count: 0,
                }),
            );
        }
    }

    // Load conversation from MongoDB
    match state.db.get_conversation(&session_id).await {
        Ok(conversation) => {
            debug!(
                "✅ Loaded conversation with {} messages",
                conversation.messages().len()
            );

            let mut session_messages = Vec::new();
            for (index, message) in conversation.messages().iter().enumerate() {
                // Extract role as string (removing Debug formatting)
                let role_str = format!("{:?}", message.role);
                
                // Get message content
                let content = message.as_concat_text();
                
                // Create timestamp (we don't have access to creation time from GooseMessage,
                // so we'll leave it as None for now)
                let timestamp = None;

                session_messages.push(SessionMessage {
                    role: role_str,
                    content,
                    timestamp,
                    message_index: index,
                });
            }

            let total_count = session_messages.len();
            info!(
                "✅ Retrieved {} messages for session {}",
                total_count, session_id
            );

            (
                StatusCode::OK,
                Json(SessionMessagesResponse {
                    session_id,
                    messages: session_messages,
                    total_count,
                }),
            )
        }
        Err(e) => {
            error!(
                "❌ Failed to load conversation for session {} from MongoDB: {}",
                session_id, e
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(SessionMessagesResponse {
                    session_id,
                    messages: vec![],
                    total_count: 0,
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

// EXISTING: Agent-level streaming - refactored to use shared logic
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

    // Set processing status
    set_agent_status(&state, &session_id, "agent-streaming", "processing");

    // Use shared message processing logic
    let (conversation, _user_message) = match process_user_message(&session_id, &request.message, &state).await {
        Ok(result) => result,
        Err((_status, json_response)) => {
            // Set error status
            set_agent_status(&state, &session_id, "agent-streaming", "error");
            
            // Convert JSON error to SSE error format
            let error_data = serde_json::to_string(&StreamEvent {
                event_type: "error".to_string(),
                content: json_response.0,
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
        state.clone(), // Pass state for status updates
        session_id.clone(),
        conversation,
    );

    Sse::new(agent_stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}



// Shared function for processing user messages (used by both streaming and non-streaming)
async fn process_user_message(
    session_id: &str,
    message_content: &str,
    state: &AppState,
) -> Result<(Conversation, GooseMessage), (StatusCode, Json<serde_json::Value>)> {
    // Verify session exists in MongoDB
    match state.db.get_session(session_id).await {
        Ok(Some(_)) => {
            debug!("✅ Session found in MongoDB: {}", session_id);
        }
        Ok(None) => {
            warn!("❌ Session not found in MongoDB: {}", session_id);
            return Err((
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({
                    "error": "Session not found",
                    "message": format!("Session {} not found in MongoDB", session_id)
                })),
            ));
        }
        Err(e) => {
            error!(
                "❌ Failed to check session {} in MongoDB: {}",
                session_id, e
            );
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "Database error",
                    "message": "Failed to query session from MongoDB"
                })),
            ));
        }
    }

    // Save user message to MongoDB
    debug!("💾 Saving user message to MongoDB...");
    let user_message = GooseMessage::user().with_text(message_content);
    if let Err(e) = state.db.add_message(session_id, &user_message).await {
        error!("❌ Failed to save user message to MongoDB: {}", e);
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": "Failed to save message",
                "message": "Could not save user message to database"
            })),
        ));
    }
    debug!("✅ User message saved to MongoDB");

    // Load conversation history from MongoDB
    debug!("📖 Loading conversation history from MongoDB...");
    let conversation = match state.db.get_conversation(session_id).await {
        Ok(conv) => {
            info!(
                "✅ Loaded conversation with {} messages from MongoDB",
                conv.messages().len()
            );
            conv
        }
        Err(e) => {
            error!("❌ Failed to load conversation from MongoDB: {}", e);
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "Failed to load conversation",
                    "message": "Could not load conversation history from database"
                })),
            ));
        }
    };

    Ok((conversation, user_message))
}

// NEW: Non-streaming fire-and-forget message endpoint
async fn send_message_sync(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
    Json(request): Json<MessageRequest>,
) -> impl IntoResponse {
    info!(
        "💬 [Fire-and-Forget] Processing message for session {}",
        session_id
    );
    debug!("📝 Message content: {}", request.message);

    // Set processing status
    set_agent_status(&state, &session_id, "sync", "processing");

    // Use shared message processing logic
    let (conversation, _user_message) = match process_user_message(&session_id, &request.message, &state).await {
        Ok(result) => result,
        Err(error_response) => {
            // Set error status
            set_agent_status(&state, &session_id, "sync", "error");
            return error_response;
        }
    };

    // Process the message without streaming
    info!("🤖 [Fire-and-Forget] Starting agent processing...");
    match state.agent.reply(conversation.clone(), None, None).await {
        Ok(mut event_stream) => {
            let mut assistant_content = String::new();
            let mut tool_calls_count = 0;

            // Process all events without streaming
            while let Some(result) = event_stream.next().await {
                match result {
                    Ok(AgentEvent::Message(msg)) => {
                        // Collect assistant content and count tools
                        for message_content in &msg.content {
                            match message_content {
                                goose::conversation::message::MessageContent::Text(text) => {
                                    let role_str = format!("{:?}", msg.role);
                                    if role_str == "Assistant" {
                                        assistant_content.push_str(&text.text);
                                    }
                                },
                                goose::conversation::message::MessageContent::ToolRequest(_) => {
                                    tool_calls_count += 1;
                                    debug!("🔧 Tool call detected (non-streaming)");
                                },
                                _ => {
                                    debug!("📝 Other message content processed (non-streaming)");
                                }
                            }
                        }
                    },
                    Ok(AgentEvent::HistoryReplaced(new_messages)) => {
                        info!("🔄 Agent replaced conversation history with {} messages", new_messages.len());
                        
                        // Update MongoDB with new conversation history
                        let new_conversation = Conversation::new_unvalidated(new_messages);
                        if let Err(e) = state.db.update_conversation(&session_id, &new_conversation).await {
                            error!("❌ Failed to update conversation in MongoDB: {}", e);
                        } else {
                            debug!("✅ Updated conversation in MongoDB");
                        }
                    },
                    Ok(AgentEvent::McpNotification(_)) => {
                        debug!("🔔 MCP notification received (non-streaming)");
                    },
                    Ok(AgentEvent::ModelChange { .. }) => {
                        debug!("🔄 Model change event (non-streaming)");
                    },
                    Err(e) => {
                        error!("❌ Agent processing error: {}", e);
                        // Set error status
                        set_agent_status(&state, &session_id, "sync", "error");
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(serde_json::json!({
                                "error": "Agent processing failed",
                                "message": e.to_string(),
                                "session_id": session_id
                            })),
                        );
                    }
                }
            }

            // Save final assistant response to MongoDB
            if !assistant_content.is_empty() {
                debug!("💾 Saving assistant response to MongoDB ({} chars)", assistant_content.len());
                let assistant_message = GooseMessage::assistant().with_text(&assistant_content);
                if let Err(e) = state.db.add_message(&session_id, &assistant_message).await {
                    error!("❌ Failed to save assistant message to MongoDB: {}", e);
                    // Set error status
                    set_agent_status(&state, &session_id, "sync", "error");
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({
                            "error": "Failed to save response",
                            "message": "Could not save assistant response to database"
                        })),
                    );
                } else {
                    debug!("✅ Assistant response saved to MongoDB");
                }
            }

            // Set completed status
            set_agent_status(&state, &session_id, "sync", "completed");
            info!("✅ [Fire-and-Forget] Processing completed for session: {}", session_id);
            
            (
                StatusCode::OK,
                Json(serde_json::to_value(MessageResponse {
                    message: "Message processed successfully".to_string(),
                    session_id: session_id.clone(),
                    assistant_response: assistant_content,
                    timestamp: SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs(),
                    tool_calls_count,
                }).unwrap()),
            )
        }
        Err(e) => {
            error!("❌ Failed to start agent processing: {}", e);
            // Set error status
            set_agent_status(&state, &session_id, "sync", "error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "Failed to start agent processing",
                    "message": e.to_string(),
                    "session_id": session_id
                })),
            )
        }
    }
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

    // Set processing status
    set_agent_status(&state, &session_id, "provider-streaming", "processing");

    // Verify session exists in MongoDB
    match state.db.get_session(&session_id).await {
        Ok(Some(_)) => {
            debug!("✅ Session found in MongoDB: {}", session_id);
        }
        Ok(None) => {
            warn!("❌ Session not found in MongoDB: {}", session_id);
            // Set error status
            set_agent_status(&state, &session_id, "provider-streaming", "error");
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
            // Set error status
            set_agent_status(&state, &session_id, "provider-streaming", "error");
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
        state.clone(), // Pass state for status updates
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

// EXISTING: Agent-level event stream with status tracking
fn create_agent_stream(
    agent: Arc<Agent>,
    db: Arc<DatabaseManager>,
    state: AppState,
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

                            // Handle different message content types
                            for message_content in &msg.content {
                                match message_content {
                                    goose::conversation::message::MessageContent::Text(text) => {
                                        let event_data = StreamEvent {
                                            event_type: "message".to_string(),
                                            content: serde_json::json!({
                                                "role": format!("{:?}", msg.role),
                                                "content": text.text,
                                                "content_type": "text",
                                                "timestamp": SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs(),
                                                "session_id": session_id
                                            }),
                                        };

                                        // Collect assistant content for saving to MongoDB
                                        let role_str = format!("{:?}", msg.role);
                                        if role_str == "Assistant" {
                                            assistant_content.push_str(&text.text);
                                        }

                                        if let Ok(data) = serde_json::to_string(&event_data) {
                                            yield Ok(Event::default().data(data));
                                        }
                                    },
                                    goose::conversation::message::MessageContent::ToolRequest(req) => {
                                        debug!("🔧 Tool request: {}", req.to_readable_string());
                                        
                                        let event_data = StreamEvent {
                                            event_type: "tool_request".to_string(),
                                            content: serde_json::json!({
                                                "id": req.id,
                                                "tool_name": req.tool_call.as_ref().map(|tc| &tc.name).unwrap_or(&"unknown".to_string()),
                                                "arguments": req.tool_call.as_ref().map(|tc| &tc.arguments).unwrap_or(&serde_json::Value::Null),
                                                "content_type": "tool_request",
                                                "timestamp": SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs(),
                                                "session_id": session_id
                                            }),
                                        };

                                        if let Ok(data) = serde_json::to_string(&event_data) {
                                            yield Ok(Event::default().data(data));
                                        }
                                    },
                                    goose::conversation::message::MessageContent::ToolResponse(resp) => {
                                        debug!("🔨 Tool response: ID {}", resp.id);
                                        
                                        let event_data = StreamEvent {
                                            event_type: "tool_response".to_string(),
                                            content: serde_json::json!({
                                                "id": resp.id,
                                                "result": resp.tool_result,
                                                "is_error": resp.tool_result.is_err(),
                                                "content_type": "tool_response",
                                                "timestamp": SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs(),
                                                "session_id": session_id
                                            }),
                                        };

                                        if let Ok(data) = serde_json::to_string(&event_data) {
                                            yield Ok(Event::default().data(data));
                                        }
                                    },
                                    goose::conversation::message::MessageContent::ToolConfirmationRequest(confirmation) => {
                                        debug!("🔍 Tool confirmation request: {}", confirmation.tool_name);
                                        
                                        let event_data = StreamEvent {
                                            event_type: "tool_confirmation".to_string(),
                                            content: serde_json::json!({
                                                "id": confirmation.id,
                                                "tool_name": confirmation.tool_name,
                                                "arguments": confirmation.arguments,
                                                "needs_confirmation": true,
                                                "content_type": "tool_confirmation",
                                                "timestamp": SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs(),
                                                "session_id": session_id
                                            }),
                                        };

                                        if let Ok(data) = serde_json::to_string(&event_data) {
                                            yield Ok(Event::default().data(data));
                                        }
                                    },
                                    goose::conversation::message::MessageContent::Thinking(thinking) => {
                                        debug!("🤔 Thinking: {}", thinking.thinking);
                                        
                                        let event_data = StreamEvent {
                                            event_type: "thinking".to_string(),
                                            content: serde_json::json!({
                                                "message": thinking.thinking,
                                                "signature": thinking.signature,
                                                "content_type": "thinking",
                                                "timestamp": SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs(),
                                                "session_id": session_id
                                            }),
                                        };

                                        if let Ok(data) = serde_json::to_string(&event_data) {
                                            yield Ok(Event::default().data(data));
                                        }
                                    },
                                    goose::conversation::message::MessageContent::ContextLengthExceeded(ctx_msg) => {
                                        debug!("📉 Context length exceeded: {}", ctx_msg.msg);
                                        
                                        let event_data = StreamEvent {
                                            event_type: "context_exceeded".to_string(),
                                            content: serde_json::json!({
                                                "message": ctx_msg.msg,
                                                "content_type": "context_exceeded",
                                                "timestamp": SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs(),
                                                "session_id": session_id
                                            }),
                                        };

                                        if let Ok(data) = serde_json::to_string(&event_data) {
                                            yield Ok(Event::default().data(data));
                                        }
                                    },
                                    goose::conversation::message::MessageContent::Image(_) => {
                                        // Handle image content if needed
                                        debug!("🖼️ Image content in message");
                                    },
                                    _ => {
                                        // Handle other content types (FrontendToolRequest, RedactedThinking, etc.)
                                        debug!("📝 Other message content type");
                                    }
                                }
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
                // Set completed status
                set_agent_status(&state, &session_id, "agent-streaming", "completed");
                
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
                
                // Set error status
                set_agent_status(&state, &session_id, "agent-streaming", "error");

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
    state: AppState,
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
                // Set completed status
                set_agent_status(&state, &session_id, "provider-streaming", "completed");
                
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
                
                // Set error status
                set_agent_status(&state, &session_id, "provider-streaming", "error");

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
