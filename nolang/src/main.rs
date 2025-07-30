fn add(a: i32, b: i32) -> i32 {
    a + b
}

fn subtract(a: i32, b: i32) -> i32 {
    a - b
}

fn multiply(a: i32, b: i32) -> i32 {
    a * b
}

fn divide(a: i32, b: i32) -> Result<i32, String> {
    if b == 0 {
        Err(String::from("Error: Division by zero is not allowed."))
    } else {
        Ok(a / b)
    }
}

fn main() {
    let x = 20;
    let y = 5;
    let z = 0;

    println!("--- Basic Arithmetic Operations ---");
    println!("Numbers: x = {}, y = {}, z = {}", x, y, z);

    // Addition
    let sum = add(x, y);
    println!("{} + {} = {}", x, y, sum);

    // Subtraction
    let difference = subtract(x, y);
    println!("{} - {} = {}", x, y, difference);

    // Multiplication
    let product = multiply(x, y);
    println!("{} * {} = {}", x, y, product);

    // Division (successful case)
    match divide(x, y) {
        Ok(quotient) => println!("{} / {} = {}", x, y, quotient),
        Err(e) => println!("{}", e), // This branch won't be hit for x/y
    }

    // Division (division by zero case)
    match divide(x, z) {
        Ok(quotient) => println!("{} / {} = {}", x, z, quotient), // This branch won't be hit
        Err(e) => println!("{}", e), // This branch will be hit
    }

    // Another example with different numbers
    let a = 100;
    let b = 10;
    println!("\n--- Another Example ---");
    println!("Numbers: a = {}, b = {}", a, b);
    println!("{} * {} = {}", a, b, multiply(a, b));
    println!("{} - {} = {}", a, b, subtract(a, b));
}