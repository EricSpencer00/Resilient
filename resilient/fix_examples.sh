#!/bin/bash

# Advanced script to fix Resilient examples
# This script makes examples compatible with the Resilient parser requirements

# Display help information
show_help() {
  echo "Resilient Example Fixer"
  echo "Usage: $0 [options] [file]"
  echo ""
  echo "Options:"
  echo "  -h, --help     Show this help message"
  echo "  -a, --all      Process all examples in the examples directory"
  echo "  -v, --verbose  Show detailed information about changes"
  echo ""
  echo "If no file is specified, the script will check all files in the examples directory."
}

# Process a single file
process_file() {
  local file="$1"
  local verbose="$2"
  local backup="${file}.bak"
  
  if [ ! -f "$file" ]; then
    echo "Error: File '$file' not found"
    return 1
  fi
  
  # Create a backup
  cp "$file" "$backup"
  
  if [ "$verbose" = "true" ]; then
    echo "Processing $file..."
  fi
  
  # Fix 1: Ensure function declarations have parameters with types
  # This regex matches function declarations without parameters or with parameters missing types
  sed -E 's/fn ([a-zA-Z_][a-zA-Z0-9_]*)\(\) \{/fn \1(int dummy) {/g' "$backup" > "$file"
  
  # Fix 2: Add types to parameters that are missing them
  sed -E 's/fn ([a-zA-Z_][a-zA-Z0-9_]*)\(([a-zA-Z_][a-zA-Z0-9_]*)\) \{/fn \1(int \2) {/g' "$file" > "$backup"
  cp "$backup" "$file"
  
  # Fix 3: Add types to multiple parameters that are missing them
  sed -E 's/fn ([a-zA-Z_][a-zA-Z0-9_]*)\(([a-zA-Z_][a-zA-Z0-9_]*), ([a-zA-Z_][a-zA-Z0-9_]*)\) \{/fn \1(int \2, int \3) {/g' "$file" > "$backup"
  cp "$backup" "$file"
  
  # Fix 4: Add types to three parameters that are missing them
  sed -E 's/fn ([a-zA-Z_][a-zA-Z0-9_]*)\(([a-zA-Z_][a-zA-Z0-9_]*), ([a-zA-Z_][a-zA-Z0-9_]*), ([a-zA-Z_][a-zA-Z0-9_]*)\) \{/fn \1(int \2, int \3, int \4) {/g' "$file" > "$backup"
  cp "$backup" "$file"
  
  # Fix 5: Ensure function calls include arguments
  sed -E 's/([a-zA-Z_][a-zA-Z0-9_]*)\(\);/\1(0);/g' "$file" > "$backup"
  cp "$backup" "$file"
  
  # Fix 6: Add missing semicolons to statement endings
  sed -E 's/^([[:space:]]*)(let|return)[[:space:]]+([^;]+)$/\1\2 \3;/g' "$file" > "$backup"
  cp "$backup" "$file"
  
  # Fix 7: Ensure assert statements have proper parentheses
  sed -E 's/assert ([^(].*);/assert(\1);/g' "$file" > "$backup"
  cp "$backup" "$file"
  
  # Fix 8: Fix missing braces in live blocks
  sed -E 's/live ([^{].*)/live {\n    \1\n}/g' "$file" > "$backup"
  cp "$backup" "$file"
  
  # Fix 9: Ensure proper string concatenation with +
  sed -E 's/println\("([^"]*)" \+ ([^)]+)\);/println("\1" + \2);/g' "$file" > "$backup"
  cp "$backup" "$file"
  
  # Fix 4: Ensure main function is called at the end if it exists and is not already called
  if grep -q "fn main(" "$file" && ! grep -q "main(.*)" "$file"; then
    if [ "$verbose" = "true" ]; then
      echo "Adding main function call to $file"
    fi
    echo -e "\n// Call the main function to start execution\nmain(0);" >> "$file"
  fi
  
  if [ "$verbose" = "true" ]; then
    echo "Completed processing $file"
    echo "Changes made:"
    diff -u "$backup" "$file" | grep -v "^---" | grep -v "^+++"
    echo ""
  fi
  
  # Remove backup if not in verbose mode
  if [ "$verbose" != "true" ]; then
    rm "$backup"
  fi
  
  return 0
}

# Process all files in examples directory
process_all_examples() {
  local verbose="$1"
  local count=0
  local success=0
  
  echo "Processing all examples in examples directory..."
  
  for file in examples/*.rs new_examples/*.rs; do
    if [ -f "$file" ]; then
      count=$((count + 1))
      if process_file "$file" "$verbose"; then
        success=$((success + 1))
      fi
    fi
  done
  
  echo "Processed $count files, $success successfully fixed"
}

# Main script logic
VERBOSE="false"
ALL="false"
FILE=""

# Parse command line arguments
while [[ $# -gt 0 ]]; do
  case "$1" in
    -h|--help)
      show_help
      exit 0
      ;;
    -a|--all)
      ALL="true"
      shift
      ;;
    -v|--verbose)
      VERBOSE="true"
      shift
      ;;
    *)
      FILE="$1"
      shift
      ;;
  esac
done

# Execute based on arguments
if [ "$ALL" = "true" ]; then
  process_all_examples "$VERBOSE"
elif [ -n "$FILE" ]; then
  process_file "$FILE" "$VERBOSE"
else
  show_help
fi
