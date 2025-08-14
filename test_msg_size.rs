fn main() {
    // Test typical ASCII frame size
    let width = 60;
    let height = 25;
    let frame_size = width * height + height; // characters + newlines
    println!("ASCII frame size: {} bytes", frame_size);
    
    // Test report size
    let report_lines = 30; // estimate
    let avg_line_length = 60;
    let report_size = report_lines * avg_line_length;
    println!("Report size estimate: {} bytes", report_size);
    
    // Serialized with bincode adds overhead
    println!("With bincode overhead (2x): {} bytes", (frame_size + report_size) * 2);
}
