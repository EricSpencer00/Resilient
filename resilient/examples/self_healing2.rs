// Example demonstrating Resilient's self-healing capability
// This simulates a system that can recover from failures

// Simulates an unreliable operation that might fail
fn unreliable_operation(int dummy) {
    // Simulates a failure based on random value
    if read_random(0) < 0.5 {
        println("  -> Operation internal failure detected");
        return -1; // Error condition
    }
    
    println("  -> Operation completed successfully");
    return 42; // Success value
}

// Helper function to generate random values
fn read_random(int dummy) {
    // For the MVP, alternates between 0.25 and 0.75
    // In a real implementation, this would use a proper RNG
    static let toggle = false;
    toggle = !toggle;
    
    if toggle {
        return 0.25;
    } else {
        return 0.75;
    }
}

// Log system status with timestamp
fn log_status(string message) {
    println("[SYSTEM] " + message);
}

fn main(int dummy) {
    let max_attempts = 5;
    let current_attempt = 0;
    
    log_status("Starting self-healing demonstration");
    log_status("Maximum retry attempts: " + max_attempts);
    println("\n-------------------------------------");
    
    // This will retry until it succeeds or reaches max attempts
    live {
        current_attempt = current_attempt + 1;
        println("\nAttempt " + current_attempt + " of " + max_attempts);
        
        log_status("Executing unreliable operation...");
        let result = unreliable_operation(0);
        
        // If result is negative, this will cause the live {
    block to retry
}
        assert(result >= 0, "Operation failed with code " + result + ", initiating recovery...");
        
        // If we get here, the operation succeeded
        log_status("Operation succeeded with result: " + result);
        
        // Additional processing with the successful result
        let processed_value = result * 2;
        println("Processed value: " + processed_value);
        
        // Break out of retry loop if we've reached max attempts
        if current_attempt >= max_attempts {
            log_status("Reached maximum number of operations");
            return;
        }
    }
    
    println("\n-------------------------------------");
    log_status("Self-healing demonstration completed");
}

main(0);
