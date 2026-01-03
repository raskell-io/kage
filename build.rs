//! Build script for Kage
//!
//! Compiles protobuf definitions when the server feature is enabled.

fn main() {
    // Compile protos if server feature is enabled
    #[cfg(feature = "server")]
    {
        let proto_files = &["proto/kage.proto"];
        let includes = &["proto/"];

        // Check if proto files exist
        if std::path::Path::new("proto/kage.proto").exists() {
            tonic_build::configure()
                .build_server(true)
                .build_client(true)
                .compile(proto_files, includes)
                .expect("Failed to compile protos");
        }
    }
}
