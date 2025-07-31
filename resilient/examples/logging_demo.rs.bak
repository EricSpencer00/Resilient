// Enhanced logging demonstration for Resilient

// Function that returns a negative value
fn negative_value(int dummy) {
    println("Returning a negative value...");
    return -10;
}

// Main function to demonstrate enhanced assertion and live block logging
fn main(int dummy) {
    println("Starting enhanced logging demonstration");
    println("---------------------------------------");
    
    // Live block will show enhanced logging messages
    live {
        println("Inside live block, calling function...");
        let result = negative_value(0);
        
        // This assertion will fail, triggering the retry mechanism
        assert(result > 0, "Value must be positive");
        
        println("This line should not execute due to the assertion failure");
    }
    
    println("---------------------------------------");
    println("Demonstration completed");
}

// Run the main function
main(0);
