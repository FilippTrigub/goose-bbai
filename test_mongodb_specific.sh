#!/bin/bash

# MongoDB-Specific Test Script for Goose API Server
set -e

API_BASE="http://localhost:3001/api/v1"
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

echo -e "${BLUE}🗺️ MongoDB Integration Test Suite${NC}"
echo "======================================="
echo "API Base: $API_BASE"
echo "Testing MongoDB session management..."
echo ""

# Check if jq is available
if ! command -v jq &> /dev/null; then
    echo -e "${YELLOW}⚠️ jq not found, installing...${NC}"
    sudo apt update && sudo apt install -y jq
fi

# Test 1: Health Check with MongoDB Status
echo -e "${BLUE}1. Testing Health Check (MongoDB Status)${NC}"
echo "curl -s '$API_BASE/health'"
HEALTH_RESPONSE=$(curl -s "$API_BASE/health")
HEALTH_STATUS=$(echo "$HEALTH_RESPONSE" | jq -r '.status' 2>/dev/null || echo "error")
DB_CONNECTED=$(echo "$HEALTH_RESPONSE" | jq -r '.database_connected' 2>/dev/null || echo "false")
STORAGE_TYPE=$(echo "$HEALTH_RESPONSE" | jq -r '.storage_type' 2>/dev/null || echo "unknown")

echo "Response: $HEALTH_RESPONSE"
echo "Status: $HEALTH_STATUS"
echo "Database Connected: $DB_CONNECTED"
echo "Storage Type: $STORAGE_TYPE"

if [ "$HEALTH_STATUS" = "ok" ] && [ "$DB_CONNECTED" = "true" ] && [ "$STORAGE_TYPE" = "mongodb" ]; then
    echo -e "${GREEN}✅ Health check PASSED - MongoDB connected${NC}"
else
    echo -e "${RED}❌ Health check FAILED - MongoDB not properly connected${NC}"
    echo "Expected: status=ok, database_connected=true, storage_type=mongodb"
    echo "Got: status=$HEALTH_STATUS, database_connected=$DB_CONNECTED, storage_type=$STORAGE_TYPE"
    exit 1
fi

# Test 2: Create Session in MongoDB
echo -e "\n${BLUE}2. Testing Session Creation (MongoDB)${NC}"
echo "curl -s -X POST '$API_BASE/sessions'"
CREATE_RESPONSE=$(curl -s -X POST "$API_BASE/sessions")
SESSION_ID=$(echo "$CREATE_RESPONSE" | jq -r '.session_id' 2>/dev/null || echo "")

echo "Response: $CREATE_RESPONSE"
echo "Session ID: $SESSION_ID"

if [ -n "$SESSION_ID" ] && [ "$SESSION_ID" != "null" ] && [ "$SESSION_ID" != "error" ]; then
    echo -e "${GREEN}✅ Session creation PASSED - ID: $SESSION_ID${NC}"
else
    echo -e "${RED}❌ Session creation FAILED${NC}"
    exit 1
fi

# Test 3: List Sessions from MongoDB (The problematic endpoint)
echo -e "\n${BLUE}3. Testing List Sessions (MongoDB Query)${NC}"
echo "curl -s '$API_BASE/sessions'"
LIST_RESPONSE=$(curl -s "$API_BASE/sessions")
LIST_STATUS=$(echo "$LIST_RESPONSE" | jq -r '.total' 2>/dev/null || echo "error")

echo "Response: $LIST_RESPONSE"
echo "Total sessions: $LIST_STATUS"

# Check if response is valid JSON
if echo "$LIST_RESPONSE" | jq . > /dev/null 2>&1; then
    SESSIONS_COUNT=$(echo "$LIST_RESPONSE" | jq '.sessions | length' 2>/dev/null || echo "0")
    TOTAL_COUNT=$(echo "$LIST_RESPONSE" | jq -r '.total' 2>/dev/null || echo "0")
    
    echo "Sessions array length: $SESSIONS_COUNT"
    echo "Total count: $TOTAL_COUNT"
    
    if [ "$SESSIONS_COUNT" -gt 0 ] && [ "$TOTAL_COUNT" -gt 0 ]; then
        echo -e "${GREEN}✅ List sessions PASSED - Found $SESSIONS_COUNT sessions${NC}"
        
        # Show first session details
        FIRST_SESSION=$(echo "$LIST_RESPONSE" | jq '.sessions[0]' 2>/dev/null || echo "{}")
        echo "First session: $FIRST_SESSION"
    else
        echo -e "${YELLOW}⚠️ List sessions returned valid JSON but no sessions found${NC}"
    fi
else
    echo -e "${RED}❌ List sessions FAILED - Invalid JSON response${NC}"
    echo "This is likely the BSON deserialization issue you mentioned"
    
    # Check if response contains binary data indicators
    if [[ "$LIST_RESPONSE" == *"data:"* ]] && [[ "$LIST_RESPONSE" == *"ab000000"* ]]; then
        echo -e "${RED}⚠️ Confirmed: Response contains BSON binary data${NC}"
        echo "The MongoDB cursor is not properly deserializing documents"
    fi
    exit 1
fi

# Test 4: Get Session Details
echo -e "\n${BLUE}4. Testing Get Session Details (MongoDB)${NC}"
echo "curl -s '$API_BASE/sessions/$SESSION_ID'"
GET_RESPONSE=$(curl -s "$API_BASE/sessions/$SESSION_ID")
GET_SESSION_ID=$(echo "$GET_RESPONSE" | jq -r '.session_id' 2>/dev/null || echo "")

echo "Response: $GET_RESPONSE"

if [ "$GET_SESSION_ID" = "$SESSION_ID" ]; then
    echo -e "${GREEN}✅ Get session details PASSED${NC}"
else
    echo -e "${RED}❌ Get session details FAILED${NC}"
    echo "Expected session_id: $SESSION_ID"
    echo "Got session_id: $GET_SESSION_ID"
fi

# Test 5: Send Message with Streaming
echo -e "\n${BLUE}5. Testing Message Streaming (MongoDB Persistence)${NC}"
echo "curl -X POST '$API_BASE/sessions/$SESSION_ID/messages' -d '{\"message\": \"Test MongoDB integration\"}'"

# Capture streaming response with timeout
STREAM_FILE="/tmp/stream_test.txt"
timeout 15 curl -s -X POST "$API_BASE/sessions/$SESSION_ID/messages" \
    -H "Content-Type: application/json" \
    -d '{"message": "Test MongoDB integration and persistence"}' \
    --no-buffer > "$STREAM_FILE" 2>&1 || true

if [ -f "$STREAM_FILE" ]; then
    STREAM_CONTENT=$(cat "$STREAM_FILE")
    echo "Stream response: $STREAM_CONTENT"
    
    if [[ "$STREAM_CONTENT" == *"data:"* ]]; then
        echo -e "${GREEN}✅ Message streaming PASSED - SSE data received${NC}"
    else
        echo -e "${RED}❌ Message streaming FAILED - No SSE data${NC}"
    fi
else
    echo -e "${RED}❌ Message streaming FAILED - No response${NC}"
fi

rm -f "$STREAM_FILE"

# Test 6: Export Session (MongoDB)
echo -e "\n${BLUE}6. Testing Session Export (MongoDB)${NC}"
echo "curl -s '$API_BASE/sessions/$SESSION_ID/export'"
EXPORT_RESPONSE=$(curl -s "$API_BASE/sessions/$SESSION_ID/export")

echo "Export response: $EXPORT_RESPONSE"

if [[ "$EXPORT_RESPONSE" == *"Session Export"* ]] || [[ "$EXPORT_RESPONSE" == *"#"* ]]; then
    echo -e "${GREEN}✅ Session export PASSED${NC}"
else
    echo -e "${RED}❌ Session export FAILED${NC}"
fi

# Test 7: List Sessions Again (After Message)
echo -e "\n${BLUE}7. Testing List Sessions After Message (MongoDB)${NC}"
LIST_AFTER_RESPONSE=$(curl -s "$API_BASE/sessions")
echo "Response: $LIST_AFTER_RESPONSE"

if echo "$LIST_AFTER_RESPONSE" | jq . > /dev/null 2>&1; then
    MESSAGE_COUNT=$(echo "$LIST_AFTER_RESPONSE" | jq '.sessions[0].message_count' 2>/dev/null || echo "0")
    echo "Message count for session: $MESSAGE_COUNT"
    
    if [ "$MESSAGE_COUNT" -gt 0 ]; then
        echo -e "${GREEN}✅ Message persistence PASSED - Session has $MESSAGE_COUNT messages${NC}"
    else
        echo -e "${YELLOW}⚠️ Message count is 0 - messages may not be persisted correctly${NC}"
    fi
else
    echo -e "${RED}❌ List sessions after message FAILED - Still getting BSON data${NC}"
fi

# Test 8: Delete Session (MongoDB)
echo -e "\n${BLUE}8. Testing Session Deletion (MongoDB)${NC}"
echo "curl -s -X DELETE '$API_BASE/sessions/$SESSION_ID'"
DELETE_RESPONSE=$(curl -s -w "%{http_code}" -X DELETE "$API_BASE/sessions/$SESSION_ID")
DELETE_STATUS=$(echo "$DELETE_RESPONSE" | tail -c 4)

echo "Delete status: $DELETE_STATUS"

if [ "$DELETE_STATUS" = "204" ]; then
    echo -e "${GREEN}✅ Session deletion PASSED${NC}"
else
    echo -e "${RED}❌ Session deletion FAILED - Status: $DELETE_STATUS${NC}"
fi

# Final verification
echo -e "\n${BLUE}9. Final Verification - List All Sessions${NC}"
FINAL_LIST=$(curl -s "$API_BASE/sessions")
echo "Final session list: $FINAL_LIST"

if echo "$FINAL_LIST" | jq . > /dev/null 2>&1; then
    FINAL_COUNT=$(echo "$FINAL_LIST" | jq '.total' 2>/dev/null || echo "0")
    echo -e "${GREEN}✅ Final verification PASSED - $FINAL_COUNT sessions remaining${NC}"
else
    echo -e "${RED}❌ Final verification FAILED - Still getting BSON data${NC}"
fi

echo -e "\n${BLUE}🏁 MongoDB Integration Test Complete${NC}"
echo "Log file: /tmp/goose_api_test.log"
