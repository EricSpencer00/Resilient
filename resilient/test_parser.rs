// Comprehensive test file for enhanced parser validation
// Tests all statement and expression types

// Global variables
static let global_counter = 0;
static let flag = true;

// Simple function with multiple parameters
fn add(int a, int b) {
    return a + b;
}

// Function with string concatenation
fn greet(string name) {
    return "Hello, " + name + "!";
}

// Function with conditional logic
fn is_even(int num) {
    if num % 2 == 0 {
        return true;
    } else {
        return false;
    }
}

// Function with live block for error recovery
fn divide_safely(int a, int b) {
    live {
        assert(b != 0, "Division by zero not allowed");
        return a / b;
    }
}

// Test all expression types
fn test_expressions(int dummy) {
    // Integer arithmetic
    let i1 = 10;
    let i2 = 20;
    let sum = i1 + i2;
    let diff = i2 - i1;
    let product = i1 * i2;
    let quotient = i2 / i1;
    
    // Boolean expressions
    let b1 = true;
    let b2 = false;
    let and_result = b1 && b2;
    let or_result = b1 || b2;
    let not_result = !b1;
    
    // Comparison operators
    let eq = i1 == i2;
    let neq = i1 != i2;
    let gt = i2 > i1;
    let lt = i1 < i2;
    let gte = i2 >= i1;
    let lte = i1 <= i2;
    
    // String operations
    let s1 = "Hello";
    let s2 = "World";
    let greeting = s1 + ", " + s2 + "!";
    
    // Function calls
    let result = add(i1, i2);
    let message = greet("Resilient");
    
    // Print results
    println("Arithmetic: " + sum + ", " + diff + ", " + product + ", " + quotient);
    println("Boolean: " + and_result + ", " + or_result + ", " + not_result);
    println("Comparison: " + eq + ", " + neq + ", " + gt + ", " + lt + ", " + gte + ", " + lte);
    println("String: " + greeting);
    println("Function calls: " + result + ", " + message);
}

// Main function to run all tests
fn main(int dummy) {
    println("Starting parser tests...");
    
    // Test variable declarations
    let x = 42;
    let name = "Parser";
    let enabled = true;
    
    // Test function calls
    let sum = add(10, 20);
    let message = greet(name);
    let even = is_even(x);
    
    // Test assertions
    assert(sum == 30, "Addition failed");
    
    // Test if statements
    if enabled {
        println("Feature enabled");
    } else {
        println("Feature disabled");
    }
    
    // Test live blocks
    live {
        let result = divide_safely(100, 0);
        println("Result: " + result);
    }
    
    // Test all expressions
    test_expressions(0);
    
    println("All parser tests completed successfully!");
}

// Run the main function
main(0);
