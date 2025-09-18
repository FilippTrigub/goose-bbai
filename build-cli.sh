#!/bin/bash

# Quick CLI build script for goose development
# Usage: ./build-cli.sh [options]
#   --release    Build in release mode
#   --watch      Auto-rebuild on file changes
#   --run        Build and run with --help
#   --debug      Build and run with debugger
#   --test       Build and run tests
#   --check      Fast compilation check only

set -e  # Exit on error

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Default values
BUILD_MODE="debug"
WATCH_MODE=false
RUN_AFTER=false
DEBUG_MODE=false
TEST_MODE=false
CHECK_ONLY=false

# Parse command line arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --release)
            BUILD_MODE="release"
            shift
            ;;
        --watch)
            WATCH_MODE=true
            shift
            ;;
        --run)
            RUN_AFTER=true
            shift
            ;;
        --debug)
            DEBUG_MODE=true
            shift
            ;;
        --test)
            TEST_MODE=true
            shift
            ;;
        --check)
            CHECK_ONLY=true
            shift
            ;;
        -h|--help)
            echo "Usage: $0 [options]"
            echo "  --release    Build in release mode"
            echo "  --watch      Auto-rebuild on file changes"
            echo "  --run        Build and run with --help"
            echo "  --debug      Build and run with debugger"
            echo "  --test       Build and run tests"
            echo "  --check      Fast compilation check only"
            echo "  -h, --help   Show this help message"
            exit 0
            ;;
        *)
            echo -e "${RED}Unknown option: $1${NC}"
            echo "Use --help for available options"
            exit 1
            ;;
    esac
done

# Check if cargo-watch is available for watch mode
if [ "$WATCH_MODE" = true ]; then
    if ! command -v cargo-watch &> /dev/null; then
        echo -e "${YELLOW}cargo-watch not found. Installing...${NC}"
        cargo install cargo-watch
    fi
fi

# Setup environment if hermit is available
if [ -f "./bin/activate-hermit" ]; then
    echo -e "${BLUE}🔧 Activating Hermit environment...${NC}"
    source ./bin/activate-hermit
fi

# Function to build CLI
build_cli() {
    local mode=$1
    echo -e "${BLUE}🔨 Building goose CLI in $mode mode...${NC}"

    if [ "$mode" = "release" ]; then
        cargo build --release -p goose-cli
        BINARY_PATH="./target/release/goose"
    else
        cargo build -p goose-cli
        BINARY_PATH="./target/debug/goose"
    fi

    if [ $? -eq 0 ]; then
        echo -e "${GREEN}✅ Build successful!${NC}"
        echo -e "${GREEN}Binary location: $BINARY_PATH${NC}"
        return 0
    else
        echo -e "${RED}❌ Build failed!${NC}"
        return 1
    fi
}

# Function to run tests
run_tests() {
    echo -e "${BLUE}🧪 Running tests for goose-cli...${NC}"
    cargo test -p goose-cli

    if [ $? -eq 0 ]; then
        echo -e "${GREEN}✅ Tests passed!${NC}"
    else
        echo -e "${RED}❌ Tests failed!${NC}"
        return 1
    fi
}

# Function to run the CLI
run_cli() {
    local binary=$1
    echo -e "${BLUE}🚀 Running goose CLI...${NC}"
    echo -e "${YELLOW}Command: $binary --help${NC}"
    echo "----------------------------------------"
    $binary --help
}

# Function to debug the CLI
debug_cli() {
    local binary=$1
    echo -e "${BLUE}🐛 Starting debugger for goose CLI...${NC}"
    echo -e "${YELLOW}Use 'break main' to set breakpoint, then 'run --help'${NC}"

    if command -v rust-lldb &> /dev/null; then
        rust-lldb $binary
    elif command -v rust-gdb &> /dev/null; then
        rust-gdb $binary
    else
        echo -e "${RED}No debugger found (rust-lldb or rust-gdb)${NC}"
        return 1
    fi
}

# Main execution
if [ "$CHECK_ONLY" = true ]; then
    echo -e "${BLUE}⚡ Running fast compilation check...${NC}"
    cargo check -p goose-cli
    if [ $? -eq 0 ]; then
        echo -e "${GREEN}✅ Code compiles successfully!${NC}"
    else
        echo -e "${RED}❌ Compilation errors found!${NC}"
        exit 1
    fi
elif [ "$WATCH_MODE" = true ]; then
    echo -e "${BLUE}👀 Starting watch mode...${NC}"
    if [ "$TEST_MODE" = true ]; then
        echo -e "${YELLOW}Auto-running tests on file changes...${NC}"
        cargo watch -c -x "test -p goose-cli"
    elif [ "$RUN_AFTER" = true ]; then
        echo -e "${YELLOW}Auto-building and running on file changes...${NC}"
        if [ "$BUILD_MODE" = "release" ]; then
            cargo watch -c -s "cargo build --release -p goose-cli && ./target/release/goose --help"
        else
            cargo watch -c -s "cargo build -p goose-cli && ./target/debug/goose --help"
        fi
    else
        echo -e "${YELLOW}Auto-building on file changes...${NC}"
        if [ "$BUILD_MODE" = "release" ]; then
            cargo watch -c -x "build --release -p goose-cli"
        else
            cargo watch -c -x "build -p goose-cli"
        fi
    fi
else
    # Regular build
    if build_cli $BUILD_MODE; then
        if [ "$BUILD_MODE" = "release" ]; then
            BINARY_PATH="./target/release/goose"
        else
            BINARY_PATH="./target/debug/goose"
        fi

        if [ "$TEST_MODE" = true ]; then
            run_tests
        fi

        if [ "$DEBUG_MODE" = true ]; then
            debug_cli $BINARY_PATH
        elif [ "$RUN_AFTER" = true ]; then
            run_cli $BINARY_PATH
        fi
    else
        exit 1
    fi
fi