#!/bin/bash
# Script to test the enhanced parser with various examples

# Set colors for terminal output
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[0;33m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

echo -e "${CYAN}Resilient Enhanced Parser Test${NC}"
echo "=============================="

# Function to run a test file and check for parser errors
run_test() {
    local file="$1"
    local description="$2"
    
    echo -e "\n${CYAN}Testing:${NC} $description"
    echo -e "${CYAN}File:${NC} $file"
    
    # Run the resilient parser on the file
    OUTPUT=$(cargo run -- "$file" 2>&1)
    
    # Check for parser errors
    if echo "$OUTPUT" | grep -q "Parser error"; then
        echo -e "${RED}Test FAILED${NC}"
        echo -e "${RED}Parser errors detected:${NC}"
        echo "$OUTPUT" | grep -A 2 "Parser error"
        return 1
    elif echo "$OUTPUT" | grep -q "Error:"; then
        echo -e "${YELLOW}Test produced runtime errors (expected for some tests):${NC}"
        echo "$OUTPUT" | grep -A 2 "Error:"
        echo -e "${GREEN}Parser test PASSED${NC}"
        return 0
    else
        echo -e "${GREEN}Test PASSED${NC}"
        return 0
    fi
}

# Make sure the test files are fixed for Resilient syntax
echo -e "\n${CYAN}Fixing example files for Resilient syntax...${NC}"
./fix_examples.sh --all

# Compile the latest code
echo -e "\n${CYAN}Compiling Resilient...${NC}"
cargo build

# Run tests
echo -e "\n${CYAN}Running parser tests...${NC}"

# Test 1: Comprehensive parser test
run_test "test_parser.rs" "Comprehensive parser test file"

# Test 2: Basic hello world
run_test "examples/hello.rs" "Basic hello world example"

# Test 3: Minimal example
run_test "examples/minimal.rs" "Minimal working example"

# Test 4: Live block example
run_test "examples/self_healing.rs" "Self-healing example with live blocks"

# Test 5: Assertions
run_test "examples/sensor_example.rs" "Sensor example with assertions"

# Test 6: Comprehensive language features
run_test "examples/comprehensive.rs" "Comprehensive language features"

echo -e "\n${CYAN}Parser tests completed${NC}"
echo "========================"
