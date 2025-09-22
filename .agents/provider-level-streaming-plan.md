# Provider-Level Token Streaming Implementation Plan

## 🔍 **Research Findings**

### **Current Goose Streaming Architecture**

**1. Provider Level (`MessageStream`):**
```rust
// From providers/base.rs - THIS IS WHAT WE WANT TO ACCESS
pub type MessageStream = Pin<
    Box<dyn Stream<Item = Result<(Option<Message>, Option<ProviderUsage>), ProviderError>> + Send>,
>;

// Provider trait:
async fn stream(
    &self,
    system: &str,
    messages: &[Message],
    tools: &[Tool],
) -> Result<MessageStream, ProviderError>
```

**2. OpenAI Implementation Shows Token-Level Streaming:**
```rust
// From providers/formats/openai.rs - EXACTLY what we need!
// Shows actual token chunks:
"content": "I'll run both"        // First chunk
"content": " `ls` commands in a"   // Second chunk  
"content": " single turn for you -" // Third chunk
```

**3. Current Agent Level (`AgentEvent`):**
```rust
// From agents/agent.rs - CURRENT IMPLEMENTATION
pub enum AgentEvent {
    Message(Message),           // Complete message chunks
    McpNotification(...),       // Tool notifications
    ModelChange { ... },        // Model switches  
    HistoryReplaced(...),       // Conversation updates
}
```

### **Key Discovery:**
**The provider-level `MessageStream` already gives us token-level streaming!** 
Each `delta.content` chunk in OpenAI responses contains partial text that builds up the complete response.

---

## 🎯 **Implementation Plan: Provider-Level Streaming Endpoint**

### **Goal:**
Create a new endpoint `/api/v1/sessions/{id}/stream` that bypasses the Agent layer and streams directly from the Provider's `MessageStream`.

### **Architecture:**

```
Current Flow:
Provider::stream() -> Agent::reply() -> AgentEvent -> API SSE
                         ↑
                   (aggregates chunks)

New Flow:
Provider::stream() -> API SSE (direct)
       ↑
   (raw token chunks)
```

### **Implementation Steps:**

#### **Phase 1: New Streaming Endpoint**

**1. Add New Route**
```rust
// In api_server.rs
.route("/api/v1/sessions/{id}/stream", post(stream_direct_from_provider))
```

**2. Create Provider-Level Stream Handler**
```rust
async fn stream_direct_from_provider(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
    Json(request): Json<MessageRequest>,
) -> impl IntoResponse {
    // 1. Verify session exists
    // 2. Load conversation from MongoDB
    // 3. Add user message to conversation
    // 4. Get provider directly (bypass Agent)
    // 5. Call provider.stream() directly
    // 6. Stream raw MessageStream chunks as SSE
    // 7. Save complete response to MongoDB when done
}
```

#### **Phase 2: Provider Access Pattern**

**Challenge:** Currently, Provider is wrapped inside Agent. We need direct access.

**Solution Options:**

**Option A: Expose Provider from Agent**
```rust
// Add to Agent
impl Agent {
    pub fn get_provider(&self) -> Arc<dyn Provider> {
        self.provider.clone()
    }
}
```

**Option B: Create Provider Directly**
```rust
// In api_server.rs - create provider same way as Agent does
let provider = goose::providers::create(&provider_name, model_config)?;
```

**Option C: Extract from AppState**
```rust
// Store provider separately in AppState
struct AppState {
    agent: Arc<Agent>,
    provider: Arc<dyn Provider>,  // Direct provider access
    db: Arc<DatabaseManager>,
}
```

#### **Phase 3: MessageStream to SSE Conversion**

```rust
fn create_provider_stream(
    provider: Arc<dyn Provider>,
    system: String,
    messages: Vec<Message>,
    tools: Vec<Tool>,
) -> impl Stream<Item = Result<Event, Infallible>> {
    async_stream::stream! {
        match provider.stream(&system, &messages, &tools).await {
            Ok(mut message_stream) => {
                let mut accumulated_content = String::new();
                
                while let Some(result) = message_stream.next().await {
                    match result {
                        Ok((Some(message), usage)) => {
                            // This is a token/chunk from the provider
                            let chunk_content = message.as_concat_text();
                            accumulated_content.push_str(&chunk_content);
                            
                            // Stream the chunk immediately
                            let event_data = serde_json::json!({
                                "type": "token",
                                "content": chunk_content,
                                "accumulated": accumulated_content.clone(),
                                "timestamp": SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs()
                            });
                            
                            yield Ok(Event::default().data(serde_json::to_string(&event_data).unwrap()));
                        },
                        Ok((None, usage)) => {
                            // Usage info or other metadata
                            if let Some(usage_info) = usage {
                                let event_data = serde_json::json!({
                                    "type": "usage",
                                    "content": usage_info
                                });
                                yield Ok(Event::default().data(serde_json::to_string(&event_data).unwrap()));
                            }
                        },
                        Err(e) => {
                            let event_data = serde_json::json!({
                                "type": "error",
                                "content": e.to_string()
                            });
                            yield Ok(Event::default().data(serde_json::to_string(&event_data).unwrap()));
                            break;
                        }
                    }
                }
                
                // Save complete response to MongoDB
                let final_message = GooseMessage::assistant().with_text(&accumulated_content);
                // TODO: Save to MongoDB
                
                // Send completion
                let event_data = serde_json::json!({
                    "type": "complete",
                    "content": "Token streaming complete",
                    "total_length": accumulated_content.len()
                });
                yield Ok(Event::default().data(serde_json::to_string(&event_data).unwrap()));
            },
            Err(e) => {
                let event_data = serde_json::json!({
                    "type": "error",
                    "content": e.to_string()
                });
                yield Ok(Event::default().data(serde_json::to_string(&event_data).unwrap()));
            }
        }
    }
}
```

#### **Phase 4: Tool Integration**

**Challenge:** How to handle tool calls in token streaming?

**Solutions:**
1. **Separate endpoints** - `/stream` for text only, `/messages` for tools
2. **Mode parameter** - `?mode=tokens` vs `?mode=events`
3. **Hybrid approach** - stream tokens for text, emit events for tools

---

## 🏗️ **Implementation Strategy**

### **Approach: Non-Breaking Addition**

**Keep Current System:**
- `/api/v1/sessions/{id}/messages` - Current agent-level streaming (unchanged)
- All existing functionality preserved

**Add New Endpoint:**
- `/api/v1/sessions/{id}/stream` - New provider-level token streaming

### **Benefits:**
1. **No breaking changes** - existing API clients continue working
2. **Direct provider access** - raw token-level streaming
3. **Performance** - bypasses Agent processing overhead
4. **Flexibility** - clients can choose streaming granularity

### **File Changes Required:**

**1. `api_server.rs` - Add new endpoint:**
```rust
.route("/api/v1/sessions/{id}/stream", post(stream_direct_from_provider))
```

**2. `api_server.rs` - Update AppState:**
```rust
struct AppState {
    agent: Arc<Agent>,
    provider: Arc<dyn Provider>,  // Direct provider access
    db: Arc<DatabaseManager>,
}
```

**3. `api_server.rs` - New handler function:**
```rust
async fn stream_direct_from_provider(...) -> impl IntoResponse
fn create_provider_token_stream(...) -> impl Stream<...>
```

### **API Design:**

**New Endpoint:**
```
POST /api/v1/sessions/{id}/stream
Body: {"message": "Your prompt here"}
Response: text/event-stream

SSE Events:
data: {"type": "token", "content": "Hello", "accumulated": "Hello"}
data: {"type": "token", "content": " there", "accumulated": "Hello there"}
data: {"type": "token", "content": "!", "accumulated": "Hello there!"}
data: {"type": "complete", "content": "Token streaming complete"}
```

**Existing Endpoint (Unchanged):**
```
POST /api/v1/sessions/{id}/messages  
// Continues to work with agent-level events
```

---

## 🚧 **Implementation Complexity**

### **Low Complexity:**
- ✅ **Provider already supports streaming** - `provider.stream()` exists
- ✅ **Token-level data available** - OpenAI format shows chunk-level content
- ✅ **Non-breaking approach** - add new endpoint alongside existing

### **Medium Complexity:**
- ⚠️ **Provider access** - need to expose provider from Agent or create separately
- ⚠️ **Tool handling** - decide how to handle tool calls in token stream
- ⚠️ **State management** - accumulate content for MongoDB storage

### **Considerations:**
- **Provider compatibility** - not all providers support streaming
- **Error boundaries** - partial token streams need graceful error handling
- **Performance** - direct provider access should be faster
- **Client complexity** - clients need to handle token accumulation

---

## 🎯 **Recommended Implementation Order**

1. **Phase 1:** Add provider to AppState (Option B above)
2. **Phase 2:** Implement basic token streaming endpoint
3. **Phase 3:** Add MongoDB persistence for token streams
4. **Phase 4:** Handle tool calls gracefully (probably emit as single events)
5. **Phase 5:** Add error handling and usage tracking

### **Success Criteria:**
- ✅ **Real-time token streaming** working
- ✅ **Existing endpoints** unchanged and working
- ✅ **MongoDB persistence** for token-streamed messages
- ✅ **Error handling** for unsupported providers
- ✅ **Usage tracking** maintained

---

## 📋 **Next Steps**

1. **Implement Option B** - Create provider separately in AppState
2. **Add `/stream` endpoint** with basic token streaming
3. **Test with OpenAI provider** (known to support fine-grained streaming)
4. **Add MongoDB persistence** for accumulated content
5. **Test against existing `/messages` endpoint** to ensure no regression

**This approach gives you direct access to provider-level token streaming while keeping the existing system intact.**
