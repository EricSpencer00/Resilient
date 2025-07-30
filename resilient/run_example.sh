#!/bin/bash

# Run Resilient examples script
# Usage: ./run_example.sh [example_name] [--typecheck]

# Default values
EXAMPLE="test"
TYPECHECK=""

# Parse arguments
for arg in "$@"
do
  if [ "$arg" == "--typecheck" ]; then
    TYPECHECK="--typecheck"
  else
    EXAMPLE="$arg"
  fi
done

# Add .rs extension if not provided
if [[ ! "$EXAMPLE" == *.rs ]]; then
  EXAMPLE="${EXAMPLE}.rs"
fi

# If file doesn't exist, try with examples/ prefix
if [ ! -f "$EXAMPLE" ]; then
  if [ -f "examples/$EXAMPLE" ]; then
    EXAMPLE="examples/$EXAMPLE"
  else
    echo "Error: Example file '$EXAMPLE' not found"
    echo "Available examples:"
    ls -1 examples/*.rs | sort
    exit 1
  fi
fi

# Check if example is one of the working examples
WORKING_EXAMPLES=("examples/minimal.rs" "examples/comprehensive.rs" "examples/sensor_example2.rs" "examples/self_healing2.rs")
FOUND=0

for we in "${WORKING_EXAMPLES[@]}"; do
  if [ "$EXAMPLE" == "$we" ]; then
    FOUND=1
    break
  fi
done

if [ $FOUND -eq 0 ]; then
  echo "Warning: This example may not work with the current parser."
  echo "Working examples are: minimal, comprehensive, sensor_example2, self_healing2"
  echo "Proceeding anyway..."
fi

# Build and run
echo "Running $EXAMPLE $TYPECHECK"
cargo run -- $TYPECHECK $EXAMPLE
