// Example demonstrating parameter handling in Resilient

// Function with empty parameter list using the required syntax
fn print_hello(int dummy) {
    println("Hello from a function with a dummy parameter!");
    println("This shows the Resilient parameter system!");
}

// Function with meaningful parameters
fn with_params(int x, string message) {
    println("Function with params called with: " + x + " and message: " + message);
}

// Main function with the required dummy parameter
fn main(int dummy) {
    println("Starting program...");
    println("-------------------");
    println("Calling function with dummy parameter:");
    print_hello(0);
    println("-------------------");
    println("Calling function with meaningful parameters:");
    with_params(42, "Hello, Resilient!");
    println("-------------------");
    println("Program completed successfully!");
}

// Start the program
main(0);
