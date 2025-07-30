#!/bin/bash

# Script to convert functions without parameters to Resilient-compatible format
# Usage: ./convert_functions.sh input_file.rs output_file.rs

if [ "$#" -ne 2 ]; then
    echo "Usage: $0 input_file.rs output_file.rs"
    exit 1
fi

INPUT_FILE="$1"
OUTPUT_FILE="$2"

if [ ! -f "$INPUT_FILE" ]; then
    echo "Error: Input file '$INPUT_FILE' not found."
    exit 1
fi

# Create a temporary file
TMP_FILE=$(mktemp)

# Replace function declarations without parameters with the required format
cat "$INPUT_FILE" | sed -E 's/fn ([a-zA-Z_][a-zA-Z0-9_]*)\(\) \{/fn \1(int dummy) {/g' > "$TMP_FILE"

# Replace function calls without parameters
cat "$TMP_FILE" | sed -E 's/([a-zA-Z_][a-zA-Z0-9_]*)\(\);/\1(0);/g' > "$OUTPUT_FILE"

# Clean up
rm "$TMP_FILE"

echo "Conversion complete. Output written to $OUTPUT_FILE"
echo "Please verify the file contents before running."
