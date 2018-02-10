mod codec;
mod handshake;

pub use self::codec::APCodec;
pub use self::handshake::handshake;

use futures::{Future, Sink, Stream};
use std::io;
use std::net::ToSocketAddrs;
use tokio_core::net::TcpStream;
use tokio_core::reactor::Handle;
use tokio_io::codec::Framed;
use protobuf::{self, Message};

use authentication::Credentials;
use version;

pub type Transport = Framed<TcpStream, APCodec>;

pub fn connect<A: ToSocketAddrs>(addr: A, handle: &Handle) -> Box<Future<Item = Transport, Error = io::Error>> {
    let addr = addr.to_socket_addrs().unwrap().next().unwrap();
    let socket = TcpStream::connect(&addr, handle);
    let connection = socket.and_then(|socket| {
        handshake(socket)
    });

    Box::new(connection)
}

pub fn authenticate(transport: Transport, credentials: Credentials, device_id: String)
    -> Box<Future<Item = (Transport, Credentials), Error = io::Error>>
{
    use protocol::authentication::{APWelcome, ClientResponseEncrypted, CpuFamily, Os};

    let packet = protobuf_init!(ClientResponseEncrypted::new(), {
        login_credentials => {
            username: credentials.username,
            typ: credentials.auth_type,
            auth_data: credentials.auth_data,
        },
        system_info => {
            cpu_family: CpuFamily::CPU_UNKNOWN,
            os: Os::OS_UNKNOWN,
            system_information_string: format!("librespot_{}_{}", version::short_sha(), version::build_id()),
            device_id: device_id,
        },
        version_string: version::version_string(),
    });

    let cmd = 0xab;
    let data = packet.write_to_bytes().unwrap();

    Box::new(transport.send((cmd, data)).and_then(|transport| {
        transport.into_future().map_err(|(err, _stream)| err)
    }).and_then(|(packet, transport)| {
        match packet {
            Some((0xac, data)) => {
                let welcome_data: APWelcome =
                    protobuf::parse_from_bytes(data.as_ref()).unwrap();

                let reusable_credentials = Credentials {
                    username: welcome_data.get_canonical_username().to_owned(),
                    auth_type: welcome_data.get_reusable_auth_credentials_type(),
                    auth_data: welcome_data.get_reusable_auth_credentials().to_owned(),
                };

                Ok((transport, reusable_credentials))
            }

            Some((0xad, _)) => {
                error!("An error occurred whilst authenticating. This is likely due an incorrect password or the use of a free account.");
                panic!("Authentication failed")
            },
            Some((cmd, _)) => panic!("Unexpected packet {:?}", cmd),
            None => panic!("EOF"),
        }
    }))
}
