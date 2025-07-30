// Comprehensive example demonstrating all key features of Resilient
// This includes: live blocks, assertions, static variables, etc.

// ----- System status monitoring -----

// Track system state
static let system_online = false;
static let error_count = 0;
static let max_allowed_errors = 3;

// Initialize the system and return status
fn initialize_system(int dummy) {
    system_online = true;
    error_count = 0;
    println("System initialized and online");
    return true;
}

// Log a message with timestamp
fn log_message(string level, string message) {
    println("[" + level + "] " + message);
}

// ----- Sensor simulation -----

// Generate a simulated sensor reading
fn read_sensor(int sensor_id) {
    // Simulate occasional sensor failure
    if get_random_value(0) < 0.2 {
        log_message("ERROR", "Sensor " + sensor_id + " failed to read");
        return -1;
    }
    
    // Generate a value between 0 and 100
    return get_random_value(0) * 100;
}

// Simple random value generator (0.0 to 1.0)
fn get_random_value(int dummy) {
    static let counter = 0;
    counter = (counter + 1) % 10;
    return counter / 10.0;
}

// ----- Data processing -----

// Process sensor data and check against thresholds
fn process_data(int sensor_id, float value, float threshold) {
    // Verify the reading is valid
    assert(value >= 0, "Invalid sensor reading: " + value);
    
    // Check against threshold
    if value > threshold {
        log_message("WARNING", "Sensor " + sensor_id + " above threshold: " + value);
        
        // Critical condition check
        if value > threshold * 2 {
            log_message("CRITICAL", "Sensor " + sensor_id + " at critical level!");
            error_count = error_count + 1;
        }
    } else {
        log_message("INFO", "Sensor " + sensor_id + " reading normal: " + value);
    }
    
    return value;
}

// ----- Main control loop -----

// Monitor a specific sensor
fn monitor_sensor(int sensor_id, float threshold) {
    log_message("INFO", "Starting monitoring for sensor " + sensor_id);
    
    // System invariant check
    assert(system_online, "System must be online before monitoring");
    
    // This is a resilient block that will retry on failure
    live {
        // Get reading from sensor
        let reading = read_sensor(sensor_id);
        
        // Validate the reading (will cause retry if invalid)
        assert(reading >= 0, "Failed to get valid reading from sensor " + sensor_id);
        
        // Process the valid reading
        process_data(sensor_id, reading, threshold);
        
        // Check if we've had too many errors
        if error_count >= max_allowed_errors {
            log_message("FATAL", "Too many errors detected. Shutting down.");
            system_online = false;
            return -1;
        }
    }
    
    return 0;
}

// Main entry point
fn main(int dummy) {
    log_message("INFO", "Starting Resilient demonstration");
    
    // Initialize the system
    initialize_system(0);
    
    // Define monitoring parameters
    let sensor_count = 3;
    let threshold = 70.0;
    
    // Monitor each sensor in a loop
    let i = 0;
    while i < sensor_count {
        monitor_sensor(i, threshold);
        i = i + 1;
    }
    
    log_message("INFO", "Demonstration completed");
}

// Run the program
main(0);
