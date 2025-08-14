use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize, Debug)]
enum StreamMessage {
    PreviewFrame { ascii: String, captured: usize, total: usize },
    StatusUpdate { message: String },
    Complete,
}

fn main() {
    let msg = StreamMessage::Complete;
    let data = bincode::serialize(&msg).unwrap();
    println\!("Complete serializes to {} bytes: {:?}", data.len(), data);
    println\!("As hex: {:02x?}", data);
}
