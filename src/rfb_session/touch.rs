#![allow(dead_code)]
use super::rfb_messages::{
    ToServerMessage,
    PointerEventArgs,
    Point,
};

use tokio::sync::mpsc::Sender;
use tokio::sync::oneshot;

use tokio::io::AsyncReadExt;
use tokio::fs::{
    OpenOptions
};
use tokio_fd::AsyncFd;
use std::mem;
use std::convert::TryFrom;
use std::os::unix::io::AsRawFd;
use super::{
    RfbSessionError,
    RfbSessionErrorKind,
};

use std::convert::TryInto;

#[repr(C)]
#[derive(Debug)]
struct InputEvent {
    seconds: i32,
    micro_seconds: i32,
    event_type: u16,
    code: u16,
    value: i32,
}

impl InputEvent {
    fn from_buffer(buffer: &[u8]) -> InputEvent {
        InputEvent {
            seconds: i32::from_ne_bytes(buffer[0..4].try_into().unwrap()),
            micro_seconds: i32::from_ne_bytes(buffer[4..8].try_into().unwrap()),
            event_type: u16::from_ne_bytes(buffer[8..10].try_into().unwrap()),
            code: u16::from_ne_bytes(buffer[10..12].try_into().unwrap()),
            value: i32::from_ne_bytes(buffer[12..16].try_into().unwrap()),
        }
    }
}

pub async fn run(stop: oneshot::Receiver<bool>, output_sender: Sender<ToServerMessage>) {
    let _ = handle_input(stop, output_sender).await;
}

const EVENTS_BUFFER_SIZE: usize = 64 * mem::size_of::<InputEvent>();
const EV_ABS:u16 = 3;
const EV_KEY:u16 = 1;

const CODE_ABS_X:u16 = 0;
const CODE_ABS_Y:u16 = 1;
const CODE_ABS_MT_POSITION_X:u16 = 53;
const CODE_ABS_MT_POSITION_Y:u16 = 54;
const CODE_BTN_TOUCH:u16 = 330;

#[allow(unused_variables)]
async fn handle_input(stop_rx: oneshot::Receiver<bool>, output_sender: Sender<ToServerMessage>) -> Result<(), RfbSessionError> {
    //let input_device = "/dev/input/by-path/platform-soc:firmware:touchscreen-event";
    let input_device_name = "/dev/input/event0";
    let events_input_file = OpenOptions::new().read(true).open(input_device_name).await.unwrap();
    let mut events_input = AsyncFd::try_from(events_input_file.as_raw_fd())?;
    let mut x:u16 = 0;
    let mut y:u16 = 0;

    let result =tokio::select! {
        _ = stop_rx => Err(RfbSessionError(RfbSessionErrorKind::SessionClosedByServer)),
        _ = async {
            loop {
                let mut input_buffer: [u8; EVENTS_BUFFER_SIZE] = [0; EVENTS_BUFFER_SIZE];

                let bytes_read = events_input.read(&mut input_buffer[..]).await.unwrap();
                let events_count = bytes_read / mem::size_of::<InputEvent>();
                
                for event_index in 0..events_count {
                    let the_event = InputEvent::from_buffer(&input_buffer[event_index*mem::size_of::<InputEvent>()..]);

                    match the_event {
                        InputEvent{event_type: EV_ABS, code: CODE_ABS_MT_POSITION_X, value, ..} => x = value as u16,
                        InputEvent{event_type: EV_ABS, code: CODE_ABS_MT_POSITION_Y, value, ..} => y = value as u16,
                        InputEvent{event_type: EV_KEY, code: CODE_BTN_TOUCH, value: 1, ..} => 
                            output_sender.send(ToServerMessage::PointerEvent(PointerEventArgs{button_mask:1, location: Point{x, y}})).await.unwrap(),
                        InputEvent{event_type: EV_KEY, code: CODE_BTN_TOUCH, value: 0, ..} => 
                            output_sender.send(ToServerMessage::PointerEvent(PointerEventArgs{button_mask:0, location: Point{x, y}})).await.unwrap(),
                        _ => ()
                    }
                }
            }
        } => Err(RfbSessionError(RfbSessionErrorKind::SessionClosedByServer))
    };
    
    result
}
