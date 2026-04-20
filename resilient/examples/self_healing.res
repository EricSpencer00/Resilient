
// Example demonstrating Resilient's self-healing capability

// Simulates an unreliable operation that might fail
fn unreliable_operation() {
    // Simulates a failure 50% of the time
    if read_random() < 0.5 {
        return -1; // Error condition
    }
    
    return 42; // Success value
}

// Helper function to generate random values
fn read_random() {
    // For the MVP, alternates between 0.25 and 0.75
    // In a real implementation, this would use a RNG
    static let toggle = false;
    toggle = !toggle;
    
    if toggle {
        return 0.25;
    } else {
        return 0.75;
    }
}

fn main() {
    let max_attempts = 5;
    let current_attempt = 0;
    
    // This will retry until it succeeds or reaches max attempts
    live {
        current_attempt = current_attempt + 1;
        println("Attempt " + current_attempt + " of " + max_attempts);
        
        let result = unreliable_operation();
        
        // If result is negative, this will cause the live block to retry
        assert(result >= 0, "Operation failed, retrying...");
        
        // If we get here, the operation succeeded
        println("Operation succeeded with result: " + result);
        
        // Break out of retry loop
        if current_attempt >= max_attempts {
            println("Reached maximum retry attempts");
            return;
        }
    }
}

main();
