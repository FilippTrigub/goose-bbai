# Rust Goose API Implementation Plan

## Executive Summary

This document outlines a comprehensive plan to **extend the existing Goose CLI** with REST API capabilities by adding a new web server mode. Rather than building a standalone API service, we will enhance the Goose CLI application to include an API server command that provides session management with MongoDB persistence and real-time streaming responses using Server-Sent Events (SSE). 

The implementation follows the existing Goose architecture patterns found in `goose-bbai` and extends the current `goose web` command to provide full REST API functionality alongside the existing WebSocket-based interface.

**Key Features:**
- **Extended Goose CLI** with new `goose api-server` command
- Session management with MongoDB persistence (no hybrid storage - database is sole source of truth)
- Real-time streaming responses via SSE using existing `AgentEvent` system
- Direct integration with Goose's agent system (no subprocess execution)
- Full compatibility with existing Goose CLI commands and session management
- High-performance async architecture leveraging existing Axum infrastructure

**Architecture Approach:**
Instead of creating a separate API service, we extend the existing Goose CLI with a new web server mode that:
1. Reuses the existing Axum infrastructure from `commands/web.rs`
2. Integrates directly with Goose's agent system and session management
3. Adds REST endpoints alongside the existing WebSocket interface
4. Provides MongoDB storage for API sessions (authentication is out of scope)

---

## Immediate Implementation Plan (Stage 1)

Based on the refined requirements, here is the specific implementation plan for immediate delivery:

### Step 1: Scaffold the `api-server` Command
- **Location**: Create `crates/goose-cli/src/commands/api_server.rs`
- **CLI Integration**: Add `ApiServer` variant to main CLI enum in `main.rs`
- **Arguments**: Add `--port` (default 3000), `--host` (default localhost), `--cors-origins`
- **Reuse Pattern**: Follow the existing `web.rs` command structure and Axum setup

### Step 2: Implement Health Check Endpoint
- **Endpoint**: `GET /api/v1/health`
- **Response**: `200 OK` with JSON `{"status": "ok", "timestamp": "...", "version": "..."}`
- **Purpose**: Basic connectivity and server status verification

### Step 3: Implement Session Create Endpoint (File-based Stub)
- **Endpoint**: `POST /api/v1/sessions`
- **Request Body**: Empty (no authentication for now)
- **Response**: `201 Created` with JSON `{"session_id": "<uuid>"}`
- **Implementation**:
  - Generate UUID for session ID using `uuid` crate
  - Create empty `.jsonl` file in sessions directory (`~/.local/share/goose/sessions/{session_id}.jsonl`)
  - Use existing `SessionStore` patterns from `web.rs`

### Step 4: Implement Streaming Message Endpoint
- **Endpoint**: `POST /api/v1/sessions/{id}/messages`
- **Request Body**: JSON `{"message": "User prompt here"}`
- **Response**: `200 OK` with `Content-Type: text/event-stream`
- **Implementation**:
  - Load conversation history from `{session_id}.jsonl` using existing `SessionStore`
  - Call `agent.reply(messages, session_config, None)` to get `AgentEvent` stream
  - Convert each `AgentEvent` to Server-Sent Event format:
    - `AgentEvent::Message(msg)` → `data: {"type": "message", "content": "..."}\n\n`
    - `AgentEvent::McpNotification(notif)` → `data: {"type": "tool_notification", "content": {...}}\n\n`
    - `AgentEvent::HistoryReplaced(msgs)` → `data: {"type": "history_update", "messages": [...]}\n\n`
  - Handle client disconnections gracefully
  - Save updated conversation to `.jsonl` file after streaming completes

### Step 5: Required Dependencies
Add to `Cargo.toml` (likely already present):
```toml
uuid = { version = "1.0", features = ["v4"] }
axum = { version = "0.7", features = ["sse"] }
tokio = { version = "1.0", features = ["full"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
```

### Step 6: File Structure Changes
```
crates/goose-cli/src/
├── main.rs (add ApiServer to CLI enum)
├── commands/
│   ├── mod.rs (add api_server module)
│   ├── web.rs (existing - reference for patterns)
│   └── api_server.rs (new)
```

### Step 7: Expected API Usage
```bash
# Start the API server
goose api-server --port 3001

# Health check
curl http://localhost:3001/api/v1/health

# Create session
curl -X POST http://localhost:3001/api/v1/sessions

# Send message with streaming response
curl -X POST http://localhost:3001/api/v1/sessions/{session_id}/messages \\
  -H "Content-Type: application/json" \\
  -d '{"message": "Hello, can you help me?"}' \\
  --no-buffer
```

This implementation delivers a working API server immediately while MongoDB integration is deferred to Stage 2.


## 1. Technology Research & Recommendations

### 1.1 Database SDKs for Rust

Based on research, MongoDB will be used:

#### MongoDB Choice:
1. **mongodb** crate (Official)
   - Officially supported MongoDB Rust driver
   - Full async API with tokio support
   - Built-in BSON support
   - Comprehensive feature set

**Decision:** The official `mongodb` crate for its balance of performance, safety, and ease of use.

### 1.2 Streaming Response Capabilities

Rust web frameworks provide excellent support for Server-Sent Events (SSE) with `text/event-stream`:

#### Web Framework Choice:
1. **Axum** (Selected)
   - Built on tokio/hyper stack
   - Native SSE support via `axum::response::sse`
   - Excellent performance and ergonomics
   - Part of the tokio ecosystem
   - Already used in existing `goose web` command

**Decision:** Axum for its balance of performance, ergonomics, and ecosystem integration.

---

## 2. Phased Implementation Strategy

### 2.1 Analysis of Existing `goose web` Command

After analyzing `crates/goose-cli/src/commands/web.rs`, we discovered that Goose already has web server capabilities:

#### Current Web Interface Features:
- **Axum-based web server** with WebSocket support
- **Agent integration** - Direct use of `Agent` struct and `agent.reply()` streaming
- **Session management** - Integration with existing `.jsonl` session files
- **Real-time communication** via WebSockets
- **CORS support** for web client access
- **Static file serving** for HTML/CSS/JS assets

#### Key Infrastructure Already Present:
```rust
// From commands/web.rs - shows existing patterns to extend:

#[derive(Clone)]
struct AppState {
    agent: Arc<Agent>,                    // Direct agent integration
    sessions: SessionStore,               // In-memory session cache
    cancellations: CancellationStore,     // Task cancellation support
}

// Existing streaming implementation:
async fn process_message_streaming(
    agent: &Agent,
    session_messages: Arc<Mutex<Conversation>>,
    session_file: std::path::PathBuf,
    content: String,
    sender: Arc<Mutex<futures::stream::SplitSink<WebSocket, Message>>>,
) -> Result<()> {
    // Uses agent.reply() which returns AgentEvent stream
    match agent.reply(messages.clone(), Some(session_config), None).await {
        Ok(mut stream) => {
            while let Some(result) = stream.next().await {
                match result {
                    Ok(AgentEvent::Message(message)) => {
                        // Handle message streaming
                    }
                    Ok(AgentEvent::McpNotification(_)) => {
                        // Handle tool notifications  
                    }
                    Ok(AgentEvent::HistoryReplaced(new_messages)) => {
                        // Handle history updates
                    }
                }
            }
        }
    }
}
```

---

## 3. Revised Implementation Plan (Two-Stage Approach)

### Stage 1: Core API Server & Streaming Endpoint (Immediate Priority)

This stage focuses on creating the server and implementing the essential streaming functionality without a database dependency.

#### Phase 1: Basic API Server Setup (Week 1)
- [x] Scaffold the `api-server` command
  - Create new `api-server.rs` module within `crates/goose-cli/src/commands/`
  - Define new `ApiServer` struct and CLI arguments (e.g., `--port`) using `clap`
  - Extend main `Goose` CLI enum in `main.rs` to include `api-server` command
  - Reuse Axum server setup from existing `web.rs` command

#### Phase 2: Health Check Endpoint (Week 1)
- [x] Implement basic health check
  - Add `GET /api/v1/health` endpoint to Axum router
  - Return `200 OK` JSON response: `{"status": "ok"}`

#### Phase 3: Session Create Endpoint - Stubbed (Week 1-2)
- [x] Create session endpoint (temporary file-based)
  - Create `POST /api/v1/sessions` endpoint
  - Generate unique session ID (UUID)
  - Create temporary empty session file (`{session_id}.jsonl`) in local sessions directory
  - Work with existing `SessionStore`
  - Return JSON response containing *only* the new `session_id`

#### Phase 4: Streaming Message Endpoint (Week 2)
- [x] Implement core streaming functionality
  - Create `POST /api/v1/sessions/{id}/messages` endpoint
  - Accept user's message in JSON request body
  - Load conversation history using existing `SessionStore` and session ID from URL path
  - Call `agent.reply()` method to get stream of `AgentEvent`s
  - Wrap stream in Axum `sse::Sse` response handler
  - Stream each `AgentEvent` as properly formatted Server-Sent Event

---

### Stage 2: MongoDB-Backed Session Management (Deferred)

This stage is not part of the initial implementation but is documented for future work.

#### Phase 1: MongoDB Integration (Future)
- [ ] Implement MongoDB connector
  - Add official `mongodb` crate as dependency
  - Create database connection module with connection pool
  - Make MongoDB connection URL configurable

#### Phase 2: Schema Design (Future)
- [ ] Design and implement collection schemas
  - Define BSON document structure for `sessions` collection
  - Define BSON document structure for `messages` collection
  - Handle relationships and indexing

#### Phase 3: Database-Backed Endpoints (Future)
- [ ] Refactor session endpoints for MongoDB
  - Update `POST /api/v1/sessions` to create session document in MongoDB
  - Update `POST /api/v1/sessions/{id}/messages` to read/write from database
  - Remove dependency on local `.jsonl` files for API sessions

#### Phase 4: Full Session Management API (Future)
- [ ] Build out complete session management
  - Implement `GET /api/v1/sessions` - list sessions
  - Implement `GET /api/v1/sessions/{id}` - get session details
  - Implement `DELETE /api/v1/sessions/{id}` - delete session
  - All endpoints backed by MongoDB

---

## 4. API Endpoint Design

### 4.1 Stage 1 Endpoints (Immediate)

```
# Health Check
GET    /api/v1/health
# Returns: 200 OK, {"status": "ok"}

# Session Management (Stubbed with local files)
POST   /api/v1/sessions
# Body: (empty)
# Returns: 201 Created, {"session_id": "uuid-goes-here"}

# Messaging with Streaming
POST   /api/v1/sessions/{id}/messages
# Body: {"message": "User's prompt"}
# Returns: 200 OK, Content-Type: text/event-stream
```

### 4.2 Stage 2 Endpoints (Future - MongoDB-backed)

```
GET    /api/v1/sessions            # List all sessions
GET    /api/v1/sessions/{id}       # Get session details and history
DELETE /api/v1/sessions/{id}       # Delete session
GET    /api/v1/sessions/{id}/export   # Export session as markdown
```

---

## 5. Key Advantages of CLI Extension Approach

### 5.1 Leveraging Existing Infrastructure
- **Reuse proven patterns** from `commands/web.rs`
- **Direct agent integration** - no subprocess overhead
- **Existing session compatibility** - API sessions work with `goose session` commands (in Stage 2)
- **Built-in configuration** - uses existing `goose configure` setup

### 5.2 Architecture Benefits
- **Single binary deployment** - no separate API service needed
- **Consistent behavior** - same agent, provider, and extension system
- **Simplified debugging** - unified logging and error handling
- **Native performance** - no IPC or network overhead between CLI and API

### 5.3 Development Efficiency
- **Familiar codebase** - extends existing Goose patterns
- **Incremental enhancement** - builds on working foundation
- **Shared dependencies** - leverages existing Cargo.toml setup
- **Unified testing** - can test API and CLI together

---

## 6. Implementation Architecture

### 6.1 Extended CLI Command Structure

```
goose
├── session [existing]
├── run [existing]  
├── web [existing] - WebSocket-based interface
└── api-server [new] - REST API server
    ├── --port 3000
    ├── --database-url mongodb://... (Stage 2)
    └── --cors-origins https://...
```

### 6.2 Storage Architecture

**Stage 1 (Temporary):**
```
┌─────────────────┐    ┌──────────────────┐    ┌─────────────────┐
│   CLI Commands  │───▶│   .jsonl files   │───▶│   File System   │
│   (session,run) │◀───│   (existing)     │◀───│   ~/.local/...  │
└─────────────────┘    └──────────────────┘    └─────────────────┘
                                │
                                ▼
┌─────────────────┐    ┌──────────────────┐    
│   API Clients   │───▶│   REST API       │    
│   (HTTP/SSE)    │◀───│   (temporary     │    
└─────────────────┘    │    file-based)   │    
                        └──────────────────┘    
```

**Stage 2 (Future - MongoDB):**
```
┌─────────────────┐                            ┌─────────────────┐
│   CLI Commands  │───────────────────────────▶│   File System   │
│   (session,run) │◀───────────────────────────│   ~/.local/...  │
└─────────────────┘                            └─────────────────┘
                                
┌─────────────────┐    ┌──────────────────┐    ┌─────────────────┐
│   API Clients   │───▶│   REST API       │───▶│   MongoDB       │
│   (HTTP/SSE)    │◀───│   (database      │◀───│   (sole source) │
└─────────────────┘    │    backed)       │    └─────────────────┘
                        └──────────────────┘    
                                │
                                ▼
                        ┌──────────────────┐
                        │   Agent System   │
                        │   (shared)       │
                        └──────────────────┘
```

### 6.3 Reusing Existing Infrastructure

The implementation leverages the existing Goose infrastructure:

- **Axum Server Setup**: Reuse patterns from `commands/web.rs`
- **Agent Integration**: Direct use of the `Agent` struct and `agent.reply()` streaming
- **Session Management**: Stage 1 uses existing `SessionStore` and `.jsonl` files
- **Event System**: Leverage existing `AgentEvent` types for streaming

### 6.4 Server-Sent Events Implementation

The streaming endpoint will:
- Accept HTTP POST requests with JSON message payload
- Convert `AgentEvent` stream to SSE format
- Maintain persistent connection for real-time updates
- Handle client disconnections gracefully

### 6.5 File Structure

```
crates/goose-cli/src/commands/
├── mod.rs (updated to include api_server)
├── web.rs (existing)
└── api_server.rs (new)
```

---

## 7. Implementation Benefits

### 7.1 Immediate Deliverables (Stage 1)
- **Working REST API server** extending Goose CLI
- **Real-time streaming** of agent responses via SSE
- **Session creation** with unique IDs
- **Health monitoring** endpoint
- **No database dependencies** for quick deployment

### 7.2 Future Capabilities (Stage 2)
- **Scalable session storage** with MongoDB
- **Full session management** API (list, get, delete)
- **Database-backed persistence** for production use
- **Session export** functionality

### 7.3 Production Readiness
- **Scalable Architecture** - Built on proven Axum/Tokio foundation
- **Security** - Inherits Goose's existing security patterns and validation
- **Monitoring** - Unified logging and metrics with existing Goose telemetry
- **Deployment** - Single binary with API server mode

This phased approach provides immediate value with Stage 1 while setting up a clear path for production-scale deployment with Stage 2's MongoDB integration.
