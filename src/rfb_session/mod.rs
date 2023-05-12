use std::any::Any;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::net::tcp::{
    OwnedReadHalf,
    OwnedWriteHalf,
};
use tokio::io::AsyncWriteExt;

use std::convert::TryFrom;
use std::sync::Arc;
use tokio::sync::{
    Mutex,
    mpsc::{
        channel,
        Sender,
        Receiver,
    },
    oneshot,
};

mod rfb_messages;
mod touch;

use rfb_messages::{
    ToServerMessage,
    RfbSecurityType,
    RfbEncodingType,
    FrameUpdateRequestArgs,
    FromServerCommands,
    Point,
    Rect,
    Size,
};

mod decode;

use super::screen::Screen;

#[repr(C)]
#[derive(Debug)]
pub struct PixelFormat {
    bits_per_pixel: u8,
    depth: u8,
    big_endian: bool,
    true_color: bool,
    red_max: u16,
    green_max: u16,
    blue_max: u16,
    red_shift: u8,
    green_shift: u8,
    blue_shift: u8,
    padding: [u8; 3],
}

impl PixelFormat {
    pub fn decode(buffer: &[u8]) -> PixelFormat {
        PixelFormat {
            bits_per_pixel: buffer[0],
            depth: buffer[1],
            big_endian: buffer[2] != 0,
            true_color: buffer[3] != 0,
            red_max: u16::from_be_bytes(<[u8; 2]>::try_from(&buffer[4..6]).unwrap()),
            green_max: u16::from_be_bytes(<[u8; 2]>::try_from(&buffer[6..8]).unwrap()),
            blue_max: u16::from_be_bytes(<[u8; 2]>::try_from(&buffer[8..10]).unwrap()),
            red_shift: buffer[10],
            green_shift: buffer[11],
            blue_shift: buffer[12],
            padding: [0; 3],
        }
    }
}

#[derive(Debug)]
#[allow(dead_code)]
struct ServerInfo {
    frame_buffer_width: u16,
    frame_buffer_height: u16,
    pixel_format: PixelFormat,
    name: String,
}

pub async fn run(connection: TcpStream, screen: Arc<Mutex<Screen>>) -> Result<(), RfbSessionError> {
    let (output_sender, output_receiver): (Sender<ToServerMessage>, Receiver<ToServerMessage>) = channel(10);
    let (input_stream, output_stream) = connection.into_split();
    let (stop_touch_tx, stop_touch_rx) = oneshot::channel();
    let (stop_ping_tx, stop_ping_rx) = oneshot::channel();
    let touch_output_sender = output_sender.clone();
    let ping_output_sender = output_sender.clone();

    let from_server_thread = tokio::spawn(async move { from_server_thread(input_stream, output_sender, screen).await });
    let to_server_thread = tokio::spawn(async move { to_server_thread(output_stream, output_receiver).await });
    let touch_input_thread = tokio::spawn(async move { touch::run(stop_touch_rx, touch_output_sender).await });
    let ping_server_thread = tokio::spawn(async move { ping_server_thread(stop_ping_rx, ping_output_sender).await });

    to_server_thread.await?;
    from_server_thread.await?;

    _ = stop_touch_tx.send(true);
    touch_input_thread.await?;

    _ = stop_ping_tx.send(true);
    ping_server_thread.await?;

    Ok(())
}

async fn to_server_thread(mut output_stream: OwnedWriteHalf, mut output_receiver: Receiver<ToServerMessage>) {
    loop {
        let m = output_receiver.recv().await.expect("output_receiver.recv");

        if let ToServerMessage::Terminate = m {
            break;
        }

        let buffer = m.encode();
        
        if let Err(e) = output_stream.write(&buffer[..]).await {
            println!("Error {:?} while writing to server", e);
            break;
        }
    }
}

async fn ping_server_thread(stop_rx: oneshot::Receiver<bool>, output_sender: Sender<ToServerMessage>) {
    tokio::select! {
        _ = async {
            loop {
                tokio::time::sleep(Duration::from_secs(5*60)).await;
                let _ = output_sender.send(ToServerMessage::SetCurText("".to_string())).await;
            };
        } => { },
        _ = stop_rx => { },
    };
}

struct FromServerThread<'a> {
    reader: &'a mut OwnedReadHalf,
    sender: &'a Sender<ToServerMessage>,
    screen: &'a mut Screen,
    server_info: Option<ServerInfo>,
    same_pixel_format: bool,
}

async fn from_server_thread(mut input_stream: OwnedReadHalf, output_sender: Sender<ToServerMessage>, screen: Arc<Mutex<Screen>>) {
    let mut screen = screen.as_ref().lock().await;
    let mut fst = FromServerThread::new(&mut input_stream, &output_sender, &mut screen);

    if let Err(e) = fst.initialize_protocol().await {
        println!("Protocol initialization failed: {:?}", e);
    }

    if let Err(e) = fst.refresh_screen().await {
        println!("Session terminated {:?}", e);
    }

    output_sender.send(ToServerMessage::Terminate).await.unwrap();
}

impl FromServerThread<'_> {

    fn new<'a>(reader: &'a mut OwnedReadHalf, sender: &'a Sender<ToServerMessage>, screen: &'a mut Screen) -> FromServerThread<'a> {
        FromServerThread {
            reader,
            sender,
            screen,
            server_info: None,
            same_pixel_format: false,
        }
    }

    async fn initialize_protocol(&mut self) -> Result<(), RfbSessionError> {
        let mut protocol_version: [u8; 12] = [0; 12];

        let count = self.read(&mut protocol_version).await?;
        if count != 12 {
            return Err(RfbSessionError(RfbSessionErrorKind::ServerProtocolVersion))
        }

        self.sender.send(ToServerMessage::ProtocolVersion).await?;

        let _ = self.get_server_supported_security_options().await?;
        self.sender.send(ToServerMessage::Security(RfbSecurityType::None)).await?;

        self.get_security_result().await?;

        self.sender.send(ToServerMessage::ClientInit(true)).await?;
        self.server_info = Some(self.get_server_info().await?);
        self.same_pixel_format = self.is_same_pixel_format();

        self.sender.send(ToServerMessage::SetEncoding(vec![RfbEncodingType::HexTile, RfbEncodingType::Raw])).await?;

        Ok(())
    }

    async fn refresh_screen(&mut self) -> Result<(), RfbSessionError> {
        self.sender.send(ToServerMessage::FrameUpdateRequest(
            FrameUpdateRequestArgs {
                incremental: false,
                rect: Rect {
                    location: Point{x: 0, y: 0},
                    size: Size{
                        width: self.screen.xres() as u16,
                        height: self.screen.yres() as u16
                    }
                }
            }
        )).await?;

        loop {
            let mut command_buffer: [u8; 2] = [0; 2];

            self.read(&mut command_buffer[..]).await?;
            let command = <u16>::from_be_bytes(command_buffer);

            match FromServerCommands::new(command)? {
               
                FromServerCommands::FrameUpdate => {
                    self.frame_update().await?;

                    // Send incremental frame refresh command to get the next frame update
                    
                    self.sender.send(ToServerMessage::FrameUpdateRequest(
                        FrameUpdateRequestArgs { incremental: true,
                            rect: Rect {
                                location: Point{x: 0, y: 0},
                                size: Size{
                                    width: self.screen.xres() as u16,
                                    height: self.screen.yres() as u16
                                }
                            }
                        }
                    )).await?;
                }
            }
        }
    }

    async fn get_server_supported_security_options(&mut self) -> Result<Vec<u8>, RfbSessionError> {
        let mut buffer: [u8; 1]= [0; 1];

        self.read(&mut buffer[..]).await?;
        let count = buffer[0];

        if count == 0 {
            let error_message = self.get_string_from_server().await?;

            return Err(RfbSessionError(RfbSessionErrorKind::ServerError(error_message)));
        }

        let mut security_options = vec![0; count as usize];
        self.read(security_options.as_mut_slice()).await?;

        Ok(security_options)
    }

    async fn get_security_result(&mut self) -> Result<(), RfbSessionError> {
        let mut buffer: [u8; 4] = [0; 4];

        self.read(&mut buffer[..]).await?;
        let result = u32::from_be_bytes(buffer);

        if result != 0 {
            let error_message = self.get_string_from_server().await?;

            return Err(RfbSessionError(RfbSessionErrorKind::ServerError(error_message)));
        }
        
        Ok(())
    }

    async fn get_server_info(&mut self) -> Result<ServerInfo, RfbSessionError> {
        let mut buffer: [u8; 2+2+16] = [0; 20];

        self.read(&mut buffer[..]).await?;

        let width = u16::from_be_bytes(<[u8; 2]>::try_from(&buffer[0..2]).unwrap());
        let height = u16::from_be_bytes(<[u8; 2]>::try_from(&buffer[2..4]).unwrap());
        let pixel_format = PixelFormat::decode(&buffer[4..20]);
        let name = self.get_string_from_server().await?;

        Ok(ServerInfo{
            frame_buffer_width: width,
            frame_buffer_height: height,
            pixel_format,
            name
        })
    }

    async fn get_string_from_server(&mut self) -> Result<String, RfbSessionError> {
        let mut count_buffer: [u8; 4] = [0; 4];

        self.read(&mut count_buffer).await?;
        let count = i32::from_be_bytes(count_buffer);

        assert!(count < 1024);
        let mut message_bytes = vec![0; count as usize];

        self.read(message_bytes.as_mut_slice()).await?;
        let message = String::from_utf8(message_bytes).unwrap();

        Ok(message)
    }
}

#[derive(Debug)]
pub enum RfbSessionErrorKind {
    IoError(std::io::Error),
    OtherError(Box<dyn Any + Send + 'static>),
    SendError(tokio::sync::mpsc::error::SendError<ToServerMessage>),
    ServerProtocolVersion,
    ServerError(String),
    InvalidServerCommand(u16),
    InvalidEncoding(i32),
    SessionClosedByServer,
    JoinError,
}

#[derive(Debug)]
pub struct RfbSessionError(RfbSessionErrorKind);

impl std::error::Error for RfbSessionError {
    fn description(&self) -> &str {
        match &self.0 {
            RfbSessionErrorKind::ServerProtocolVersion => "server protocol != 12 bytes",
            RfbSessionErrorKind::IoError(_) => "IoError",
            RfbSessionErrorKind::SendError(_) => "SendError",
            RfbSessionErrorKind::OtherError(_) => "Another error",
            RfbSessionErrorKind::ServerError(_) => "Server error",
            RfbSessionErrorKind::InvalidServerCommand(_) => "Invalid server command",
            RfbSessionErrorKind::InvalidEncoding(_) => "Invalid encoding",
            RfbSessionErrorKind::SessionClosedByServer => "Session closed by server",
            RfbSessionErrorKind::JoinError => "Join error",
        }
    }
}

impl std::fmt::Display for RfbSessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{:?}", self.0)
    }
}

impl std::convert::From<Box<dyn Any + Send + 'static>> for RfbSessionError {
    fn from(err: Box<dyn Any + Send + 'static>) -> RfbSessionError {
        RfbSessionError(RfbSessionErrorKind::OtherError(err))
    }
}

impl std::convert::From<std::io::Error> for RfbSessionError {
    fn from(err: std::io::Error) -> RfbSessionError {
        RfbSessionError(RfbSessionErrorKind::IoError(err))
    }
}

impl std::convert::From<tokio::sync::mpsc::error::SendError<ToServerMessage>> for RfbSessionError {
    fn from(err: tokio::sync::mpsc::error::SendError<ToServerMessage>) -> Self {
        RfbSessionError(RfbSessionErrorKind::SendError(err))
    }
}

impl std::convert::From<tokio::task::JoinError> for RfbSessionError {
    fn from(_: tokio::task::JoinError) -> Self {
        RfbSessionError(RfbSessionErrorKind::JoinError)
    }
}
