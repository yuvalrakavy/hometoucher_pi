
use crate::rfb_session::{
    RfbSessionError,
    RfbSessionErrorKind,
};

#[derive(Debug)]
pub struct Point {
    pub x: u16,
    pub y: u16,
}

#[derive(Debug, Clone, Copy)]
pub struct Size {
    pub width: u16,
    pub height: u16,
}

#[derive(Debug)]
pub struct Rect {
    pub location: Point,
    pub size: Size,
}

#[derive(Debug)]
pub struct FrameUpdateRequestArgs {
    pub incremental: bool,
    pub rect: Rect,
}

#[derive(Debug)]
pub struct PointerEventArgs {
    pub button_mask: u8,
    pub location: Point,
}

#[derive(Clone, Copy, Debug)]
pub enum RfbEncodingType {
    Raw = 0,
    HexTile = 5,
}

#[derive(Clone, Copy, Debug)]
#[allow(dead_code)]
pub enum RfbSecurityType {
    Invalid = 0,
    None = 1,
    VncAuthentication = 2,
}

pub enum FromServerCommands {
    FrameUpdate = 0,
}

#[derive(Debug)]
pub enum ToServerMessage {
    ProtocolVersion,
    Security(RfbSecurityType),
    ClientInit(bool),
    SetEncoding(Vec<RfbEncodingType>),
    FrameUpdateRequest(FrameUpdateRequestArgs),
    PointerEvent(PointerEventArgs),
    SetCurText(String),
    Terminate,
}

use ToServerMessage::*;

impl ToServerMessage {
    pub fn encode(&self) -> Vec<u8> {
        match self {
            ProtocolVersion => Vec::from("RFB 003.008\n".as_bytes()),
            Security(security_type) => vec![*security_type as u8],
            ClientInit(shared) => vec![if *shared { 1 } else { 0} ],
            SetEncoding(encodings) => {
                let mut result = vec![2, 0];
                result.extend_from_slice(&(encodings.len() as u16).to_be_bytes());

                for encoding in encodings.iter() {
                    result.extend_from_slice(&(*encoding as i32).to_be_bytes());
                }
                result
            },
            FrameUpdateRequest(FrameUpdateRequestArgs {
                incremental,
                rect: Rect {
                    location: Point{x, y},
                    size: Size{width, height},
                }
            }) => {
                let mut result = vec![3, if *incremental { 1 } else { 0 }];
                result.extend_from_slice(&x.to_be_bytes());
                result.extend_from_slice(&y.to_be_bytes());
                result.extend_from_slice(&width.to_be_bytes());
                result.extend_from_slice(&height.to_be_bytes());
                result
            },
            PointerEvent(PointerEventArgs{
                button_mask,
                location: Point{x, y}
            }) => {
                let mut result = vec![5, *button_mask];
                result.extend_from_slice(&x.to_be_bytes());
                result.extend_from_slice(&y.to_be_bytes());
                result
            },
            SetCurText(text) => {
                let text_bytes = text.as_bytes();
                let mut result = vec![6, 0, 0, 0];
                result.extend_from_slice(&text_bytes.len().to_be_bytes());
                result.extend_from_slice(text_bytes);
                result
            },
            Terminate => panic!("Cannot encode terminate message")
        }
    }
}

impl FromServerCommands {
    pub fn new(command: u16) -> Result<FromServerCommands, RfbSessionError> {
        match command {
            0 => Ok(FromServerCommands::FrameUpdate),
            _ => Err(RfbSessionError(RfbSessionErrorKind::InvalidServerCommand(command))),
        }
    }
}

impl RfbEncodingType {
    pub fn new(encoding: i32) -> Result<RfbEncodingType, RfbSessionError> {
        match encoding {
            0 => Ok(RfbEncodingType::Raw),
            5 => Ok(RfbEncodingType::HexTile),
            _ => Err(RfbSessionError(RfbSessionErrorKind::InvalidEncoding(encoding)))
        }
    }
}