#!/bin/bash
# Script to run Resilient examples with the enhanced parser

# Set colors for terminal output
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[0;33m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

echo -e "${CYAN}Resilient Language Examples${NC}"
echo "==========================="

# Function to run an example file
run_example() {
    local file="$1"
    local description="$2"
    
    echo -e "\n${CYAN}Example:${NC} $description"
    echo -e "${CYAN}File:${NC} $file"
    echo -e "${CYAN}Code:${NC}"
    echo "------------------------"
    # Display example code with line numbers
    cat -n "$file" | sed 's/^/    /'
    echo "------------------------"
    
    # Ask if user wants to run the example
    read -p "Run this example? (y/n) " choice
    
    if [[ $choice == "y" || $choice == "Y" ]]; then
        echo -e "\n${CYAN}Running:${NC} $file"
        echo "------------------------"
        cargo run -- "$file"
        echo "------------------------"
        echo -e "${GREEN}Example completed${NC}"
    else
        echo -e "${YELLOW}Skipped${NC}"
    fi
}

# Make sure the example files are fixed for Resilient syntax
echo -e "\n${CYAN}Fixing example files for Resilient syntax...${NC}"
./fix_examples.sh --all

# Compile the latest code
echo -e "\n${CYAN}Compiling Resilient...${NC}"
cargo build

# List available examples
echo -e "\n${CYAN}Available examples:${NC}"
echo "1. Hello World (examples/hello.rs)"
echo "2. Self-healing with live blocks (examples/self_healing2.rs)"
echo "3. Sensor monitoring with assertions (examples/sensor_example2.rs)"
echo "4. Comprehensive language features (examples/comprehensive.rs)"
echo "5. Minimal example (examples/minimal.rs)"
echo "6. Run all examples"
echo "0. Exit"

# Ask which example to run
read -p "Enter your choice (0-6): " example_choice

case $example_choice in
    1)
        run_example "examples/hello.rs" "Hello World"
        ;;
    2)
        run_example "examples/self_healing2.rs" "Self-healing with live blocks"
        ;;
    3)
        run_example "examples/sensor_example2.rs" "Sensor monitoring with assertions"
        ;;
    4)
        run_example "examples/comprehensive.rs" "Comprehensive language features"
        ;;
    5)
        run_example "examples/minimal.rs" "Minimal example"
        ;;
    6)
        run_example "examples/hello.rs" "Hello World"
        run_example "examples/self_healing2.rs" "Self-healing with live blocks"
        run_example "examples/sensor_example2.rs" "Sensor monitoring with assertions"
        run_example "examples/comprehensive.rs" "Comprehensive language features"
        run_example "examples/minimal.rs" "Minimal example"
        ;;
    0)
        echo -e "${YELLOW}Exiting...${NC}"
        exit 0
        ;;
    *)
        echo -e "${RED}Invalid choice${NC}"
        exit 1
        ;;
esac

echo -e "\n${CYAN}All examples completed${NC}"
echo "========================="
