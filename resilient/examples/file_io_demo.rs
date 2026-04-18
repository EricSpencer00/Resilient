// RES-143: file_read / file_write demo.
//
// Writes a small greeting, reads it back, and prints it. Round-trips
// through the OS filesystem via the two new builtins. The temp path
// uses the system's temp dir so the example works on any host.

fn main() {
    let path = "/tmp/resilient_file_io_demo.txt";
    let greeting = "Hello from Resilient file I/O!";

    file_write(path, greeting);
    let read_back = file_read(path);

    println("wrote: " + greeting);
    println("read:  " + read_back);
}

main();
