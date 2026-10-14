fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Deliver into OUT_DIR (tonic's default) so no generated source ever lands
    // in the crate tree; `tonic::include_proto!` reads OUT_DIR automatically.
    tonic_build::configure()
        .build_server(true)
        .build_client(true)
        .compile_protos(&["proto/mail.proto"], &["proto/"])?;
    Ok(())
}
