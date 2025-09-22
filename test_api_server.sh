#!/bin/bash

# Goose API Server - Complete Test Script with Evaluation
set -e

API_BASE="http://localhost:3001/api/v1"
TEST_LOG="/tmp/goose_api_test.log"
FAILED_TESTS=0
TOTAL_TESTS=0

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Test result tracking
passed_tests=()
failed_tests=()

# Logging function
log() {
    echo "[$(date '+%Y-%m-%d %H:%M:%S')] $1" | tee -a "$TEST_LOG"
}

# Test evaluation function
evaluate_test() {
    local test_name="$1"
    local expected_status="$2"
    local actual_status="$3"
    local response="$4"
    local additional_check="$5"
    
    TOTAL_TESTS=$((TOTAL_TESTS + 1))
    
    echo -e "\n${BLUE}Test: $test_name${NC}"
    echo "Expected Status: $expected_status"
    echo "Actual Status: $actual_status"
    
    if [ "$actual_status" = "$expected_status" ]; then
        if [ -n "$additional_check" ]; then
            if eval "$additional_check"; then
                echo -e "${GREEN}✅ PASSED${NC}"
                passed_tests+=("$test_name")
            else
                echo -e "${RED}❌ FAILED (Status OK but content check failed)${NC}"
                failed_tests+=("$test_name")
                FAILED_TESTS=$((FAILED_TESTS + 1))
            fi
        else
            echo -e "${GREEN}✅ PASSED${NC}"
            passed_tests+=("$test_name")
        fi
    else
        echo -e "${RED}❌ FAILED${NC}"
        echo "Response: $response"
        failed_tests+=("$test_name")
        FAILED_TESTS=$((FAILED_TESTS + 1))
    fi
    
    log "Test: $test_name | Expected: $expected_status | Actual: $actual_status | Result: $([ "$actual_status" = "$expected_status" ] && echo PASS || echo FAIL)"
}

# Wait for server function
wait_for_server() {
    echo -e "${YELLOW}⏳ Waiting for server to be ready...${NC}"
    local max_attempts=30
    local attempt=1
    
    while [ $attempt -le $max_attempts ]; do
        if curl -s "$API_BASE/health" > /dev/null 2>&1; then
            echo -e "${GREEN}✅ Server is ready!${NC}"
            return 0
        fi
        echo "Attempt $attempt/$max_attempts - Server not ready yet..."
        sleep 2
        attempt=$((attempt + 1))
    done
    
    echo -e "${RED}❌ Server failed to start within 60 seconds${NC}"
    exit 1
}

# Start test execution
echo -e "${BLUE}🚀 Goose API Server - Complete Test Suite${NC}"
echo "==========================================="
echo "Test Log: $TEST_LOG"
echo "API Base: $API_BASE"
echo ""

# Clear previous log
> "$TEST_LOG"
log "Starting Goose API Server test suite"

# Wait for server to be ready
wait_for_server

# Test 1: Health Check
echo -e "\n${YELLOW}📋 Test 1: Health Check${NC}"
RESPONSE=$(curl -s -w "%{http_code}" "$API_BASE/health")
STATUS=$(echo "$RESPONSE" | tail -c 4)
BODY=$(echo "$RESPONSE" | head -c -4)

evaluate_test "Health Check" "200" "$STATUS" "$BODY" \
    '[[ "$BODY" == *"\"status\""* && "$BODY" == *"\"database_connected\""* ]]'

# Test 2: Create Session
echo -e "\n${YELLOW}📋 Test 2: Create Session${NC}"
RESPONSE=$(curl -s -w "%{http_code}" -X POST "$API_BASE/sessions")
STATUS=$(echo "$RESPONSE" | tail -c 4)
BODY=$(echo "$RESPONSE" | head -c -4)

# Extract session ID for later tests
if [ "$STATUS" = "201" ]; then
    SESSION_ID=$(echo "$BODY" | jq -r '.session_id' 2>/dev/null || echo "")
    log "Created session ID: $SESSION_ID"
else
    SESSION_ID=""
fi

evaluate_test "Create Session" "201" "$STATUS" "$BODY" \
    '[[ -n "$SESSION_ID" && "$SESSION_ID" != "null" && "$SESSION_ID" != "error" ]]'

# Test 3: List Sessions
echo -e "\n${YELLOW}📋 Test 3: List Sessions${NC}"
RESPONSE=$(curl -s -w "%{http_code}" "$API_BASE/sessions")
STATUS=$(echo "$RESPONSE" | tail -c 4)
BODY=$(echo "$RESPONSE" | head -c -4)

evaluate_test "List Sessions" "200" "$STATUS" "$BODY" \
    '[[ "$BODY" == *"\"sessions\""* && "$BODY" == *"\"total\""* ]]'

# Test 4: Get Session Details (if session was created)
if [ -n "$SESSION_ID" ] && [ "$SESSION_ID" != "" ]; then
    echo -e "\n${YELLOW}📋 Test 4: Get Session Details${NC}"
    RESPONSE=$(curl -s -w "%{http_code}" "$API_BASE/sessions/$SESSION_ID")
    STATUS=$(echo "$RESPONSE" | tail -c 4)
    BODY=$(echo "$RESPONSE" | head -c -4)
    
    evaluate_test "Get Session Details" "200" "$STATUS" "$BODY" \
        '[[ "$BODY" == *"$SESSION_ID"* ]]'
else
    echo -e "\n${YELLOW}📋 Test 4: Get Session Details - SKIPPED (No valid session ID)${NC}"
    log "Test 4 skipped - no valid session ID"
fi

# Test 5: Get Non-Existent Session
echo -e "\n${YELLOW}📋 Test 5: Get Non-Existent Session${NC}"
RESPONSE=$(curl -s -w "%{http_code}" "$API_BASE/sessions/non-existent-session-id")
STATUS=$(echo "$RESPONSE" | tail -c 4)
BODY=$(echo "$RESPONSE" | head -c -4)

evaluate_test "Get Non-Existent Session" "404" "$STATUS" "$BODY" \
    '[[ "$BODY" == *"not_found"* ]]'

# Test 6: Send Message (if session exists)
if [ -n "$SESSION_ID" ] && [ "$SESSION_ID" != "" ]; then
    echo -e "\n${YELLOW}📋 Test 6: Send Message with Streaming${NC}"
    
    # Capture streaming response
    TEMP_FILE="/tmp/stream_response.txt"
    timeout 10 curl -s -w "%{http_code}" -X POST "$API_BASE/sessions/$SESSION_ID/messages" \
        -H "Content-Type: application/json" \
        -d '{"message": "Hello from test script!"}' \
        --no-buffer > "$TEMP_FILE" 2>&1 || true
    
    # Extract status and body
    if [ -f "$TEMP_FILE" ]; then
        STATUS=$(tail -c 4 "$TEMP_FILE" 2>/dev/null || echo "000")
        BODY=$(head -c -4 "$TEMP_FILE" 2>/dev/null || cat "$TEMP_FILE")
        
        # For streaming, we expect 200 and event-stream data
        if [ "$STATUS" = "200" ] || [[ "$BODY" == *"data:"* ]]; then
            STATUS="200"
        fi
    else
        STATUS="000"
        BODY="No response file"
    fi
    
    evaluate_test "Send Message Streaming" "200" "$STATUS" "$BODY" \
        '[[ "$BODY" == *"data:"* || "$BODY" == *"event"* ]]'
        
    rm -f "$TEMP_FILE"
else
    echo -e "\n${YELLOW}📋 Test 6: Send Message - SKIPPED (No valid session ID)${NC}"
    log "Test 6 skipped - no valid session ID"
fi

# Test 7: Export Session (if session exists)
if [ -n "$SESSION_ID" ] && [ "$SESSION_ID" != "" ]; then
    echo -e "\n${YELLOW}📋 Test 7: Export Session${NC}"
    RESPONSE=$(curl -s -w "%{http_code}" "$API_BASE/sessions/$SESSION_ID/export")
    STATUS=$(echo "$RESPONSE" | tail -c 4)
    BODY=$(echo "$RESPONSE" | head -c -4)
    
    evaluate_test "Export Session" "200" "$STATUS" "$BODY" \
        '[[ "$BODY" == *"Session Export"* || "$BODY" == *"#"* ]]'
else
    echo -e "\n${YELLOW}📋 Test 7: Export Session - SKIPPED (No valid session ID)${NC}"
    log "Test 7 skipped - no valid session ID"
fi

# Test 8: Delete Session (if session exists)
if [ -n "$SESSION_ID" ] && [ "$SESSION_ID" != "" ]; then
    echo -e "\n${YELLOW}📋 Test 8: Delete Session${NC}"
    RESPONSE=$(curl -s -w "%{http_code}" -X DELETE "$API_BASE/sessions/$SESSION_ID")
    STATUS=$(echo "$RESPONSE" | tail -c 4)
    BODY=$(echo "$RESPONSE" | head -c -4)
    
    evaluate_test "Delete Session" "204" "$STATUS" "$BODY"
    
    # Test 8b: Verify Deletion
    echo -e "\n${YELLOW}📋 Test 8b: Verify Session Deletion${NC}"
    RESPONSE=$(curl -s -w "%{http_code}" "$API_BASE/sessions/$SESSION_ID")
    STATUS=$(echo "$RESPONSE" | tail -c 4)
    BODY=$(echo "$RESPONSE" | head -c -4)
    
    evaluate_test "Verify Session Deletion" "404" "$STATUS" "$BODY"
else
    echo -e "\n${YELLOW}📋 Test 8: Delete Session - SKIPPED (No valid session ID)${NC}"
    log "Test 8 skipped - no valid session ID"
fi

# Test 9: Error Handling - Invalid JSON
echo -e "\n${YELLOW}📋 Test 9: Error Handling - Invalid JSON${NC}"
RESPONSE=$(curl -s -w "%{http_code}" -X POST "$API_BASE/sessions/test/messages" \
    -H "Content-Type: application/json" \
    -d '{"invalid": json}' 2>/dev/null || echo "000")
STATUS=$(echo "$RESPONSE" | tail -c 4)
BODY=$(echo "$RESPONSE" | head -c -4)

evaluate_test "Invalid JSON Handling" "400" "$STATUS" "$BODY"

# Test 10: CORS Headers
echo -e "\n${YELLOW}📋 Test 10: CORS Headers${NC}"
CORS_HEADERS=$(curl -s -I "$API_BASE/health" | grep -i "access-control" | wc -l)
STATUS="200"

evaluate_test "CORS Headers Present" "200" "$STATUS" "$CORS_HEADERS headers" \
    '[[ "$CORS_HEADERS" -gt 0 ]]'

# Final Results
echo -e "\n${BLUE}===============================================${NC}"
echo -e "${BLUE}🏁 TEST RESULTS SUMMARY${NC}"
echo -e "${BLUE}===============================================${NC}"
echo -e "Total Tests: $TOTAL_TESTS"
echo -e "${GREEN}Passed: $((TOTAL_TESTS - FAILED_TESTS))${NC}"
echo -e "${RED}Failed: $FAILED_TESTS${NC}"

if [ $FAILED_TESTS -eq 0 ]; then
    echo -e "\n${GREEN}🎉 ALL TESTS PASSED! 🎉${NC}"
    log "All tests passed successfully"
    exit 0
else
    echo -e "\n${RED}❌ Some tests failed${NC}"
    echo -e "\n${YELLOW}Failed Tests:${NC}"
    for test in "${failed_tests[@]}"; do
        echo -e "${RED}  - $test${NC}"
    done
    
    echo -e "\n${GREEN}Passed Tests:${NC}"
    for test in "${passed_tests[@]}"; do
        echo -e "${GREEN}  - $test${NC}"
    done
    
    log "Test suite completed with $FAILED_TESTS failures"
    exit 1
fi
