fn main() {
    println!("cargo:rerun-if-changed=proto");
    // protox is a pure-Rust protobuf compiler: no system protoc required.
    let fds = protox::compile(["proto/protowave/v1/envelope.proto"], ["proto"])
        .expect("protobuf schemas compile");
    prost_build::Config::new()
        .compile_fds(fds)
        .expect("prost codegen");
}
