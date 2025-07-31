// Simple example of Resilient's key features
// This simulates a sensor reading system with error handling

// Simulates reading from a sensor, can fail with a negative value
fn read_sensor(int dummy) {
    // Return a random value for testing
    // In a real system, this would read from hardware
    if read_random(0) < 0.2 {
        return -1; // Simulated failure
    }
    
    return read_random(0) * 100;
}

// Helper function to generate random values
fn read_random(int dummy) {
    // For the MVP, just return a semi-random value
    // In a real implementation, this would use a proper RNG
    static let counter = 0;
    counter = (counter + 1) % 5;
    
    return (counter + 1) / 10.0;
}

// Function to check if sensor reading is valid
fn is_valid_reading(int reading) {
    return reading >= 0;
}

// Process sensor data and take appropriate action
fn process_sensor_data(int value, int threshold) {
    if value > threshold {
        println("WARNING: High sensor value: " + value);
        
        // In a real system, this might trigger an alarm or corrective action
        if value > threshold * 2 {
            println("CRITICAL: Sensor value exceeds twice the threshold!");
        }
    } else {
        println("Sensor value normal: " + value);
    }
}

// Main control loop
fn main_loop(int dummy) {
    let threshold = 50;
    let max_retries = 3;
    let current_retry = 0;
    
    // System invariant - this must never be violated
    assert(threshold > 0, "Threshold must be positive");
    
    println("Starting sensor monitoring system...");
    println("Threshold set to: " + threshold);
    
    // The live {
    block will handle recoverable errors by retrying
}
    live {
        current_retry = current_retry + 1;
        
        println("\nReading sensor (attempt " + current_retry + ")...");
        let sensor_value = read_sensor(0);
        
        // Validate the reading - if this fails, the live {
    block will retry
}
        assert(is_valid_reading(sensor_value), 
               "Invalid sensor reading detected (" + sensor_value + ")");
        
        // Process the valid reading
        process_sensor_data(sensor_value, threshold);
        
        // Exit after max retries to prevent infinite loops
        if current_retry >= max_retries {
            println("Completed " + max_retries + " readings successfully.");
            return;
        }
    }
}

// Start the main loop
fn main(int dummy) {
    main_loop(0);
}

main(0);
