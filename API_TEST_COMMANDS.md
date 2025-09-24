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
# Send a message and see streaming response
curl -X POST "$BASE/api/v1/sessions/$SESSION_ID/messages" \
  -H "Content-Type: application/json" \
  -d '{"message": "Hello, can you help me?"}' \
  --no-buffer

# Send message to test different prompts
curl -X POST "$BASE/api/v1/sessions/$SESSION_ID/messages" \
  -H "Content-Type: application/json" \
  -d '{"message": "What is the weather like today?"}' \
  --no-buffer

# Test with non-existent session (should return error)
curl -X POST "$BASE/api/v1/sessions/non-existent-id/messages" \
  -H "Content-Type: application/json" \
  -d '{"message": "This should fail"}' \
  --no-buffer

# Expected streaming response format:
# data: {"type": "message", "content": {"role": "assistant", "content": "Hello! This is the Goose API server...", "timestamp": 1698765432}}
#
# data: {"type": "complete", "content": {"message": "Response complete"}}
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

### 5. Session Deletion

#### Delete a Session
```bash
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

# 5. Send message
echo "\n5. Sending message (streaming)..."
curl -X POST "$API_BASE/sessions/$SESSION_ID/messages" \
  -H "Content-Type: application/json" \
  -d '{"message": "Hello from test script!"}' \
  --no-buffer

# 6. Export session
echo "\n\n6. Exporting session..."
curl -s "$API_BASE/sessions/$SESSION_ID/export"

# 7. Delete session
echo "\n\n7. Deleting session..."
curl -s -X DELETE "$API_BASE/sessions/$SESSION_ID" -w "HTTP Status: %{http_code}\n"

# 8. Verify deletion
echo "\n8. Verifying session deletion..."
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
