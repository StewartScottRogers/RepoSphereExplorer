//! Entry point for the service process.

fn main() -> std::io::Result<()> {
    let listener = service::bind(protocol::socket_name()?)?;
    eprintln!("listening on {}", protocol::SOCKET_NAME);
    service::run(&listener)
}
