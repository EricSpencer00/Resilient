
// Simple example of Resilient's key features
// This simulates a sensor reading system with error handling

// Simulates reading from a sensor, can fail with a negative value
fn read_sensor() {
    // Return a random value for testing
    // In a real system, this would read from hardware
    if read_random() < 0.2 {
        return -1; // Simulated failure
    }
    
    return read_random() * 100;
}

// Helper function to generate random values
fn read_random() {
    // For the MVP, just return 0.5
    // In a real implementation, this would use a RNG
    return 0.5;
}

// Function to check if sensor reading is valid
fn is_valid_reading(reading) {
    return reading >= 0;
}

// Main control loop
fn main_loop() {
    let threshold = 50;
    
    // System invariant - this must never be violated
    assert(threshold > 0, "Threshold must be positive");
    
    // The live block will handle recoverable errors
    live {
        let sensor_value = read_sensor();
        
        // Validate the reading
        assert(is_valid_reading(sensor_value), "Invalid sensor reading");
        
        // Process the valid reading
        if sensor_value > threshold {
            println("Warning: High sensor value: " + sensor_value);
        } else {
            println("Sensor value normal: " + sensor_value);
        }
    }
}

// Start the main loop
main_loop();
