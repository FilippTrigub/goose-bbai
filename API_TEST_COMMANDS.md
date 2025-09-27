# Goose API Server - Test Commands

## Prerequisites

```bash
# 1. Start the API server
cd /home/filipp/goose-bbai
cargo run -p goose-cli --bin goose -- api-server --port 3001

# 2. Open a new terminal for testing
# (Keep the server running in the first terminal)
BASE=http://localhost:3001
```

## API Test Commands

### 1. Health Check

```bash
# Basic health check
curl -X GET $BASE/api/v1/health

# Pretty print JSON response
curl -X GET $BASE/api/v1/health | jq

# Expected response:
# {
#   "status": "ok",
#   "timestamp": "1698765432",
#   "version": "1.8.0",
#   "storage_type": "in-memory"
# }
```

### 2. Session Management

#### Create a New Session
```bash
# Create session and save ID to variable
SESSION_ID=$(curl -s -X POST $BASE/api/v1/sessions | jq -r '.session_id')
echo "Created session: $SESSION_ID"

# Or create session and see full response
curl -X POST $BASE/api/v1/sessions | jq

# Expected response:
# {
#   "session_id": "550e8400-e29b-41d4-a716-446655440000"
# }
```

#### List All Sessions
```bash
# List all active sessions
curl -X GET $BASE/api/v1/sessions | jq

# Expected response:
# {
#   "sessions": [
#     {
#       "session_id": "550e8400-e29b-41d4-a716-446655440000",
#       "created_at": "2024-01-01T00:00:00Z",
#       "message_count": 0
#     }
#   ],
#   "total": 1
# }
```

#### Get Session Details
```bash
# Get specific session info (replace with actual session ID)
curl -X GET "$BASE/api/v1/sessions/$SESSION_ID" | jq

# Or with hardcoded ID for testing
curl -X GET "$BASE/api/v1/sessions/test-session-id" | jq

# Expected response for existing session:
# {
#   "session_id": "550e8400-e29b-41d4-a716-446655440000",
#   "created_at": "2024-01-01T00:00:00Z",
#   "message_count": 0
# }

# Expected response for non-existent session (404):
# {
#   "session_id": "not_found",
#   "created_at": "",
#   "message_count": 0
# }
```

### 3. Messaging with Streaming

#### Send Message with Server-Sent Events
```bash
# Send a message and see streaming response with detailed tool events
curl -X POST "$BASE/api/v1/sessions/$SESSION_ID/messages" \
  -H "Content-Type: application/json" \
  -d '{"message": "Hello, can you help me?"}' \
  --no-buffer

# Send message to test different prompts
curl -X POST "$BASE/api/v1/sessions/$SESSION_ID/messages" \
  -H "Content-Type: application/json" \
  -d '{"message": "What is the weather like today?"}' \
  --no-buffer

# Test with tool usage
curl -X POST "$BASE/api/v1/sessions/$SESSION_ID/messages" \
  -H "Content-Type: application/json" \
  -d '{"message": "Can you create a file called test.txt with some content?"}' \
  --no-buffer

# Test with non-existent session (should return error)
curl -X POST "$BASE/api/v1/sessions/invalid-session/messages" \
  -H "Content-Type: application/json" \
  -d '{"message": "This should fail"}' \
  --no-buffer

# Expected streaming response format with detailed tool events:
# data: {"type": "tool_request", "content": {"id": "req_123", "tool_name": "filesystem__write_file", "arguments": {...}, "content_type": "tool_request", "timestamp": 1698765432, "session_id": "550e8400-e29b-41d4-a716-446655440000"}}
#
# data: {"type": "tool_response", "content": {"id": "req_123", "result": {...}, "is_error": false, "content_type": "tool_response", "timestamp": 1698765433, "session_id": "550e8400-e29b-41d4-a716-446655440000"}}
#
# data: {"type": "message", "content": {"role": "Assistant", "content": "I've created the file test.txt...", "content_type": "text", "timestamp": 1698765434, "session_id": "550e8400-e29b-41d4-a716-446655440000"}}
#
# data: {"type": "complete", "content": {"message": "Response complete", "session_id": "550e8400-e29b-41d4-a716-446655440000", "storage": "mongodb"}}
```

##### Test NEW provider-level token streaming - IN DEVELOPMENT

```bash
# This one does not work yet
curl -X POST "$BASE/api/v1/sessions/$SESSION_ID/stream" \
  -H "Content-Type: application/json" \
  -d '{"message": "Write a poem"}' \
  --no-buffer
```

### 4. Session Export

#### Get Session Messages (Individual Messages)
```bash
# Get all messages for a session (structured, not concatenated)
curl -X GET "$BASE/api/v1/sessions/$SESSION_ID/messages" | jq

# Test with non-existent session
curl -X GET "$BASE/api/v1/sessions/non-existent/messages" | jq

# Expected response:
# {
#   "session_id": "550e8400-e29b-41d4-a716-446655440000",
#   "messages": [
#     {
#       "role": "User",
#       "content": "Hello, can you help me?",
#       "timestamp": null,
#       "message_index": 0
#     },
#     {
#       "role": "Assistant",
#       "content": "Hello! This is the Goose API server...",
#       "timestamp": null,
#       "message_index": 1
#     }
#   ],
#   "total_count": 2
# }
```

#### Export Session as Markdown
```bash
# Export session conversation
curl -X GET "$BASE/api/v1/sessions/$SESSION_ID/export"

# Save export to file
curl -X GET "$BASE/api/v1/sessions/$SESSION_ID/export" \
  -o "session_export_$SESSION_ID.md"

# View the exported file
cat "session_export_$SESSION_ID.md"

# Test with non-existent session
curl -X GET "$BASE/api/v1/sessions/non-existent/export"

# Expected response:
# # Session Export: 550e8400-e29b-41d4-a716-446655440000
# 
# ## Message 1 - User
# 
# Content here
# 
```

### 5. Extension Management

#### List All Extensions
```bash
# Get all extensions with their status
curl -X GET "$BASE/api/v1/extensions" | jq

# Expected response:
# {
#   "extensions": [
#     {
#       "name": "context7",
#       "display_name": null,
#       "type": "stdio",
#       "enabled": true,
#       "timeout": 10,
#       "cmd": "npx",
#       "args": ["-y", "@smithery/cli@latest", "run", "@upstash/context7-mcp", "--key", "1148243e-4547-4108-95ef-52cb0b3526c7"],
#       "bundled": null,
#       "description": null
#     }
#   ],
#   "total_count": 5
# }
```

#### Create New Extension
```bash
# Create a new stdio extension
curl -X POST "$BASE/api/v1/extensions" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "test-extension",
    "type": "stdio",
    "cmd": "npx",
    "args": ["-y", "@test/mcp-server"],
    "timeout": 30,
    "description": "Test extension for demo"
  }' | jq

# Create a built-in extension
curl -X POST "$BASE/api/v1/extensions" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "developer",
    "type": "builtin",
    "display_name": "Developer Tools",
    "timeout": 300,
    "bundled": true,
    "description": "Code editing and shell access"
  }' | jq

# Expected response:
# {
#   "message": "Extension created successfully",
#   "name": "test-extension",
#   "enabled": true
# }
```

#### Toggle Extension On/Off
```bash
# Disable an extension
curl -X PUT "$BASE/api/v1/extensions/test-extension/toggle" \
  -H "Content-Type: application/json" \
  -d '{"enabled": false}' | jq

# Enable an extension
curl -X PUT "$BASE/api/v1/extensions/test-extension/toggle" \
  -H "Content-Type: application/json" \
  -d '{"enabled": true}' | jq

# Expected response:
# {
#   "message": "Extension toggled successfully",
#   "name": "test-extension",
#   "enabled": false
# }
```

#### Remove Extension
```bash
# First disable the extension
curl -X PUT "$BASE/api/v1/extensions/test-extension/toggle" \
  -H "Content-Type: application/json" \
  -d '{"enabled": false}'

# Then remove it (only works if disabled)
curl -X DELETE "$BASE/api/v1/extensions/test-extension" -v

# Expected response: HTTP 204 No Content

# Try to remove enabled extension (should fail)
curl -X DELETE "$BASE/api/v1/extensions/context7" -v

# Expected response: HTTP 400 Bad Request with error message
```

### 7. Settings Management

#### List All Settings
```bash
# Get all configurable settings with current values and metadata
curl -X GET "$BASE/api/v1/settings" | jq

# Expected response:
# {
#   "settings": [
#     {
#       "key": "GOOSE_MODE",
#       "value": "auto",
#       "default_value": "smart_approve",
#       "description": "Agent behavior mode controlling tool usage and permissions",
#       "value_type": "string",
#       "validation": {
#         "allowed_values": ["auto", "approve", "smart_approve", "chat"]
#       },
#       "env_override": null,
#       "restart_required": false
#     }
#   ],
#   "total_count": 7
# }
```

#### Get Specific Setting
```bash
# Get a specific setting with full metadata
curl -X GET "$BASE/api/v1/settings/GOOSE_MODE" | jq
curl -X GET "$BASE/api/v1/settings/GOOSE_TEMPERATURE" | jq
curl -X GET "$BASE/api/v1/settings/GOOSE_MAX_TURNS" | jq

# Test with invalid setting
curl -X GET "$BASE/api/v1/settings/INVALID_SETTING" | jq

# Expected response for valid setting:
# {
#   "key": "GOOSE_MODE",
#   "value": "auto",
#   "default_value": "smart_approve",
#   "description": "Agent behavior mode controlling tool usage and permissions",
#   "value_type": "string",
#   "validation": {
#     "allowed_values": ["auto", "approve", "smart_approve", "chat"]
#   },
#   "env_override": null,
#   "restart_required": false
# }
```

#### Update Single Setting
```bash
# Update Goose mode
curl -X PUT "$BASE/api/v1/settings/GOOSE_MODE" \
  -H "Content-Type: application/json" \
  -d '{"value": "approve"}' | jq

# Update temperature
curl -X PUT "$BASE/api/v1/settings/GOOSE_TEMPERATURE" \
  -H "Content-Type: application/json" \
  -d '{"value": 0.8}' | jq

# Update max turns
curl -X PUT "$BASE/api/v1/settings/GOOSE_MAX_TURNS" \
  -H "Content-Type: application/json" \
  -d '{"value": 500}' | jq

# Update tool output priority
curl -X PUT "$BASE/api/v1/settings/GOOSE_CLI_MIN_PRIORITY" \
  -H "Content-Type: application/json" \
  -d '{"value": 0.2}' | jq

# Update recipe repo
curl -X PUT "$BASE/api/v1/settings/GOOSE_RECIPE_GITHUB_REPO" \
  -H "Content-Type: application/json" \
  -d '{"value": "myorg/goose-recipes"}' | jq

# Test validation errors
curl -X PUT "$BASE/api/v1/settings/GOOSE_MODE" \
  -H "Content-Type: application/json" \
  -d '{"value": "invalid_mode"}' | jq

curl -X PUT "$BASE/api/v1/settings/GOOSE_TEMPERATURE" \
  -H "Content-Type: application/json" \
  -d '{"value": 5.0}' | jq

# Expected success response:
# {
#   "message": "Setting updated successfully",
#   "key": "GOOSE_MODE",
#   "value": "approve",
#   "restart_required": false
# }
```

#### Reset Setting to Default
```bash
# Reset setting to default value
curl -X DELETE "$BASE/api/v1/settings/GOOSE_MODE" | jq
curl -X DELETE "$BASE/api/v1/settings/GOOSE_TEMPERATURE" | jq

# Expected response:
# {
#   "message": "Setting reset to default successfully",
#   "key": "GOOSE_MODE",
#   "default_value": "smart_approve"
# }
```

#### Bulk Update Settings
```bash
# Update multiple settings at once
curl -X PUT "$BASE/api/v1/settings" \
  -H "Content-Type: application/json" \
  -d '{
    "settings": {
      "GOOSE_MODE": "auto",
      "GOOSE_TEMPERATURE": 0.7,
      "GOOSE_MAX_TURNS": 1000
    }
  }' | jq

# Test mixed success/failure
curl -X PUT "$BASE/api/v1/settings" \
  -H "Content-Type: application/json" \
  -d '{
    "settings": {
      "GOOSE_MODE": "auto",
      "GOOSE_TEMPERATURE": 5.0,
      "INVALID_SETTING": "test"
    }
  }' | jq

# Expected response:
# {
#   "message": "Bulk update completed: 2/3 settings updated",
#   "success_count": 2,
#   "total_count": 3,
#   "restart_required": false,
#   "results": [
#     {
#       "key": "GOOSE_MODE",
#       "success": true,
#       "value": "auto",
#       "restart_required": false
#     },
#     {
#       "key": "GOOSE_TEMPERATURE",
#       "success": false,
#       "error": "Value must be at most 2.0"
#     }
#   ]
# }
```

### 8. Session Deletion

#### Delete a Session
# Delete the session
curl -X DELETE "$BASE/api/v1/sessions/$SESSION_ID" -v

# Expected response: HTTP 204 No Content (empty body)

# Try to delete non-existent session
curl -X DELETE "$BASE/api/v1/sessions/non-existent" -v

# Expected response: HTTP 404 Not Found

# Verify session is deleted by trying to get it
curl -X GET "$BASE/api/v1/sessions/$SESSION_ID" | jq

# Should return 404 not found
```

## Complete Test Script

```bash
#!/bin/bash
set -e

API_BASE="$BASE/api/v1"

echo "🚀 Testing Goose API Server"
echo "============================="

# 1. Health check
echo "\n1. Testing health check..."
curl -s "$API_BASE/health" | jq

# 2. Create session
echo "\n2. Creating new session..."
SESSION_ID=$(curl -s -X POST "$API_BASE/sessions" | jq -r '.session_id')
echo "Created session: $SESSION_ID"

# 3. List sessions
echo "\n3. Listing all sessions..."
curl -s "$API_BASE/sessions" | jq

# 4. Get session details
echo "\n4. Getting session details..."
curl -s "$API_BASE/sessions/$SESSION_ID" | jq

# 5. Send message (now with tool streaming)
echo "\n5. Sending message with tool streaming support..."
curl -X POST "$API_BASE/sessions/$SESSION_ID/messages" \
  -H "Content-Type: application/json" \
  -d '{"message": "Hello from test script! Can you tell me what directory I am in?"}' \
  --no-buffer

# 6. Get session messages
echo "\n\n6. Getting session messages (structured)..."
curl -s "$API_BASE/sessions/$SESSION_ID/messages" | jq

# 7. Export session
echo "\n7. Exporting session..."
curl -s "$API_BASE/sessions/$SESSION_ID/export"

# 8. List extensions
echo "\n\n8. Listing extensions..."
curl -s "$API_BASE/extensions" | jq

# 9. List settings
echo "\n9. Listing settings..."
curl -s "$API_BASE/settings" | jq

# 10. Update a setting
echo "\n10. Updating GOOSE_MODE setting..."
curl -s -X PUT "$API_BASE/settings/GOOSE_MODE" \
  -H "Content-Type: application/json" \
  -d '{"value": "auto"}' | jq

# 11. Delete session
echo "\n11. Deleting session..."
curl -s -X DELETE "$API_BASE/sessions/$SESSION_ID" -w "HTTP Status: %{http_code}\n"

# 12. Verify deletion
echo "\n12. Verifying session deletion..."
curl -s "$API_BASE/sessions/$SESSION_ID" | jq

echo "\n✅ Test completed!"
```

## Error Testing

### Test Error Handling
```bash
# Test invalid JSON
curl -X POST "$BASE/api/v1/sessions/test/messages" \
  -H "Content-Type: application/json" \
  -d '{"invalid": json}' \
  -v

# Test missing Content-Type
curl -X POST "$BASE/api/v1/sessions/test/messages" \
  -d '{"message": "test"}' \
  -v

# Test non-existent endpoints
curl -X GET "$BASE/api/v1/nonexistent" -v

# Test CORS preflight
curl -X OPTIONS "$BASE/api/v1/health" \
  -H "Access-Control-Request-Method: GET" \
  -H "Access-Control-Request-Headers: Content-Type" \
  -v
```

## Performance Testing

### Basic Load Testing
```bash
# Create multiple sessions concurrently
for i in {1..5}; do
  curl -s -X POST "$BASE/api/v1/sessions" | jq -r '.session_id' &
done
wait

# Send multiple messages to the same session
SESSION_ID=$(curl -s -X POST "$BASE/api/v1/sessions" | jq -r '.session_id')
for i in {1..3}; do
  echo "Sending message $i..."
  curl -X POST "$BASE/api/v1/sessions/$SESSION_ID/messages" \
    -H "Content-Type: application/json" \
    -d "{\"message\": \"Test message $i\"}" \
    --no-buffer &
done
wait
```

## Notes

- Replace `$BASE` with your server address if running remotely
- Use `jq` for pretty JSON formatting (install with: `sudo apt install jq`)
- The `--no-buffer` flag is important for streaming endpoints to see real-time responses
- Add `-v` flag to curl commands to see HTTP headers and status codes
- Server-Sent Events will stream data continuously until complete
