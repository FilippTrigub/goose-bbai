#!/bin/bash
# Dummy script for testing and demonstration purposes

set -e

# Color codes for output
GREEN='\033[0;32m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

echo -e "${BLUE}Starting dummy script...${NC}"

# Simple task: Display environment info
echo -e "\n${GREEN}Environment Information:${NC}"
echo "Current directory: $(pwd)"
echo "User: $(whoami)"
echo "Date: $(date)"
echo "Shell: $SHELL"

# Simple task: Count files in repository
echo -e "\n${GREEN}Repository Statistics:${NC}"
RUST_FILES=$(find . -name "*.rs" -type f 2>/dev/null | wc -l)
YAML_FILES=$(find . -name "*.yaml" -type f 2>/dev/null | wc -l)
TOTAL_FILES=$(find . -type f 2>/dev/null | wc -l)

echo "Rust files: $RUST_FILES"
echo "YAML files: $YAML_FILES"
echo "Total files: $TOTAL_FILES"

# Simple validation
echo -e "\n${GREEN}Validation:${NC}"
if [ -f "Cargo.toml" ]; then
    echo "✓ Cargo.toml found - this is a Rust project"
else
    echo "✗ Cargo.toml not found"
fi

if [ -d "crates" ]; then
    echo "✓ crates directory found"
    CRATE_COUNT=$(find crates -maxdepth 1 -type d | wc -l)
    echo "  Number of crates: $((CRATE_COUNT - 1))"
else
    echo "✗ crates directory not found"
fi

echo -e "\n${BLUE}Dummy script completed successfully!${NC}"
