// Demonstration of enhanced live block logging

// Returns a negative value to trigger assert
fn get_negative(int dummy) {
    return -5;
}

fn main(int dummy) {
    println("Starting live block demonstration");
    println("--------------------------------");
    
    // This live block will retry when the assertion fails
    live {
        println("Inside live block, getting value...");
        let value = get_negative(0);
        
        // This will fail, triggering retry with enhanced logging
        assert(value >= 0, "Value must not be negative");
        
        println("This line won't execute due to assertion failure");
    }
    
    println("--------------------------------");
    println("Demonstration completed");
}

main(0);
