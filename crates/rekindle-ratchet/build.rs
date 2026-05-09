fn main() {
    prost_build::Config::new()
        .compile_protos(&["proto/rekindle.proto"], &["proto/"])
        .expect("protobuf compilation failed");
}
