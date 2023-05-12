
use tokio::io::AsyncReadExt;
use super::{
    RfbSessionError,
    RfbSessionErrorKind,
    PixelFormat,
};
use super::rfb_messages::{
    Rect,
    Point,
    Size,
    RfbEncodingType,
};

use crate::screen::{DevicePixel, Screen};

#[derive(Debug)]
struct RectHeader {
    encoding: RfbEncodingType,
    rect: Rect,
}

trait CompactRect {
    fn get_xy(&self) -> u8;
    fn get_wh(&self) -> u8;

    fn get_x(&self) -> u16 { (self.get_xy() >> 4) as u16 }
    fn get_y(&self) -> u16 { (self.get_xy() & 0x0f) as u16 }
    fn get_w(&self) -> u16 { (self.get_wh() >> 4) as u16 + 1}
    fn get_h(&self) -> u16 { (self.get_wh() & 0x0f) as u16 + 1}

    fn get_rect(&self) -> Rect {
        Rect{location: Point{ x:self.get_x(), y:self.get_y()}, size: Size{width: self.get_w(), height: self.get_h()}}
    }
}

#[derive(Debug)]
struct ColorSubrect {
    xy: u8,
    wh: u8,
    pixel: DevicePixel,
}

impl CompactRect for ColorSubrect {
    fn get_xy(&self) -> u8 { self.xy }
    fn get_wh(&self) -> u8 { self.wh }
}

#[derive(Debug)]
struct Subrect {
    xy: u8,
    wh: u8,
}

impl CompactRect for Subrect {
    fn get_xy(&self) -> u8 { self.xy }
    fn get_wh(&self) -> u8 { self.wh }
}

impl super::FromServerThread<'_> {
    
    pub async fn frame_update(&mut self) -> Result<(), RfbSessionError> {
        let rectangle_count = self.read_u16().await?;

        for _ in 0..rectangle_count {
            let header = self.read_rect_header().await?;

            match header.encoding {
                RfbEncodingType::Raw => self.decode_raw_rect(&header).await?,
                RfbEncodingType::HexTile => self.decode_hextile_rect(&header).await?,
            }
        }

        self.screen.update();

        Ok(())
    }

    pub async fn read(&mut self, buffer: &mut [u8]) ->Result<usize, RfbSessionError> {
        let need_to_read = buffer.len();
        let mut actually_read = 0;

        while actually_read < need_to_read {
            let bytes_read = self.reader.read(&mut buffer[actually_read..]).await?;

            if bytes_read == 0 {
                return Err(RfbSessionError(RfbSessionErrorKind::SessionClosedByServer));
            }

            actually_read += bytes_read;
        }

        Ok(actually_read)
    }

    async fn decode_raw_rect(&mut self, header: &RectHeader) -> Result<(), RfbSessionError> {
        let server_bytes_per_pixel = self.bytes_per_server_pixel();
        let mut server_pixels: Vec<u8>= vec![0; (header.rect.size.height as usize) * (header.rect.size.width as usize) * server_bytes_per_pixel];
        let mut in_index:usize = 0;

        self.read(server_pixels.as_mut_slice()).await?;

        for row in 0..header.rect.size.height {
            let mut device_offset = ((row as usize) * self.screen.xres() + (header.rect.location.x as usize)) * Screen::bytes_per_pixel();

            for _ in 0..header.rect.size.width {
                let device_pixel = self.to_device_pixel(&server_pixels[in_index..]);
                in_index += server_bytes_per_pixel;

                self.screen.set_at_offset(device_offset, device_pixel);
                device_offset += Screen::bytes_per_pixel();
            }
        }
        Ok(())
    }


    async fn decode_hextile_rect(&mut self, header: &RectHeader) -> Result<(), RfbSessionError> {
        let h_tile_count = (header.rect.size.width + 15) >> 4;
        let v_tile_count = (header.rect.size.height + 15) >> 4;
        let mut hex_tile_decoder = HexTileDecoder::new(self);

        for v_tile in 0..v_tile_count {
            for h_tile in 0..h_tile_count {
                let x_offset = h_tile * 16;
                let y_offset = v_tile * 16;
                let x = header.rect.location.x + x_offset;
                let y = header.rect.location.y + y_offset;
                let tile_rect = Rect {
                    location: Point{ x, y },
                    size: Size{
                        width: if x_offset + 16 > header.rect.size.width { header.rect.size.width - x_offset } else { 16 },
                        height: if y_offset + 16 > header.rect.size.height { header.rect.size.height - y_offset } else { 16 },
                    }
                };

                hex_tile_decoder.process_tile(&tile_rect).await?;
            }
        }

        Ok(())
    }

    async fn read_u16(&mut self) -> Result<u16, RfbSessionError> {
        let mut buffer: [u8; 2] = [0; 2];

        self.read(&mut buffer[..]).await?;
        Ok(<u16>::from_be_bytes(buffer))
    }

    async fn read_i32(&mut self) -> Result<i32, RfbSessionError> {
        let mut buffer: [u8; 4] = [0; 4];

        self.read(&mut buffer[..]).await?;
        Ok(<i32>::from_be_bytes(buffer))
    }

    async fn read_rect_header(&mut self) -> Result<RectHeader, RfbSessionError> {
        let x = self.read_u16().await?;
        let y = self.read_u16().await?;
        let width = self.read_u16().await?;
        let height = self.read_u16().await?;
        let encoding = self.read_i32().await?;

        Ok(RectHeader{
            encoding: RfbEncodingType::new(encoding)?,
            rect: Rect{
                location: Point{x, y},
                size: Size{width, height}
            }
        })
    }

    fn get_server_pixel_format(&self) -> &PixelFormat {
        if let Some(ref server_info) = self.server_info {
            return &server_info.pixel_format;
        }
        panic!("No server info")
    }

    pub fn is_same_pixel_format(&self) -> bool {
        let pf = self.get_server_pixel_format();

        !pf.big_endian &&
        pf.bits_per_pixel == 16 &&
        pf.red_max == 63 && pf.red_shift == 10 &&
        pf.green_max == 127 && pf.green_shift == 4 &&
        pf.blue_max == 63 && pf.green_shift == 0
    }

    fn bytes_per_server_pixel(&self) -> usize {
        self.get_server_pixel_format().depth as usize / 8
    }

    fn to_device_pixel(&self, server_pixel: &[u8]) -> DevicePixel {
        if self.same_pixel_format {
            DevicePixel::from_value(server_pixel[0] as u16 + ((server_pixel[1] as u16) << 8))
        }
        else {
            let pf = self.get_server_pixel_format();

            if pf.depth == 32 {
                let pixel_value =  if pf.big_endian {
                    ((server_pixel[1] as u32) << 16) + ((server_pixel[2] as u32) << 8) + server_pixel[3] as u32
                } else { 
                    ((server_pixel[2] as u32) << 16) + ((server_pixel[1] as u32) << 8) + server_pixel[0] as u32
                };

                let r = ((pixel_value >> pf.red_shift) & (pf.red_max as u32)) as u8;
                let g = ((pixel_value >> pf.green_shift) & (pf.green_max as u32)) as u8;
                let b = ((pixel_value >> pf.blue_shift) & (pf.blue_max as u32)) as u8;

                DevicePixel::from_rgb(r, g, b)
            }
            else {
                panic!("Server pixel format is not supported {:#?}", pf);
            }
        }
    }
}

struct HexTileDecoder<'a, 'b> {
    fst: &'a mut super::FromServerThread<'b>,
    foreground: DevicePixel,
    background: DevicePixel,
}

impl HexTileDecoder<'_, '_> {
    fn new<'a, 'b>(fst: &'a mut super::FromServerThread<'b>) -> HexTileDecoder<'a, 'b> {
        HexTileDecoder {
            fst,
            foreground: DevicePixel::from_rgb(0, 0, 0),
            background: DevicePixel::from_rgb(0, 0, 0), 
        }
    }

    async fn process_tile(&mut self, tile_rect: &Rect) -> Result<(), RfbSessionError> {
        let server_bytes_per_pixel = self.fst.bytes_per_server_pixel();
        let mut tile_encoding: [u8; 1] = [0];

        self.fst.read(&mut tile_encoding[..]).await?;

        if tile_encoding[0] & 1 != 0 {
            let mut tile_pixels: Vec<u8> = vec![0; ((tile_rect.size.width * tile_rect.size.height) as usize) * server_bytes_per_pixel];
            let mut tile_pixels_offset = 0;

            self.fst.read(&mut tile_pixels[..]).await?;

            for row in 0..tile_rect.size.height {
                let mut device_offset = (tile_rect.location.y + row) as usize * self.fst.screen.bytes_per_row() +
                     (tile_rect.location.x as usize) * Screen::bytes_per_pixel();

                for _ in 0..tile_rect.size.width {
                    self.fst.screen.set_at_offset(device_offset, self.fst.to_device_pixel(&tile_pixels[tile_pixels_offset..]));
                    device_offset += Screen::bytes_per_pixel();
                    tile_pixels_offset += server_bytes_per_pixel;
                }
            }
        } else {
            let mut subrect_count = 0;

            if (tile_encoding[0] & 2) != 0 {
                let mut pixel_buffer: Vec<u8> = vec![0; server_bytes_per_pixel];

                self.fst.read(&mut pixel_buffer[..]).await?;
                self.background = self.fst.to_device_pixel(&pixel_buffer[..]);
            }

            if (tile_encoding[0] & 4) != 0 {
                let mut pixel_buffer: Vec<u8> = vec![0; server_bytes_per_pixel];

                self.fst.read(&mut pixel_buffer[..]).await?;
                self.foreground = self.fst.to_device_pixel(&pixel_buffer[..]);
            }

            if (tile_encoding[0] & 8) != 0 {
                let mut subrect_count_buffer: [u8; 1] = [0; 1];

                self.fst.read(&mut subrect_count_buffer[..]).await?;
                subrect_count = <u8>::from_be_bytes(subrect_count_buffer);
            }

            let subrect_are_colors = (tile_encoding[0] & 16) != 0;

            self.fill_subrect(tile_rect, &Rect{location: Point{x: 0, y: 0}, size: tile_rect.size}, self.background);

            if subrect_count > 0 {
                if subrect_are_colors {
                    for _ in 0..subrect_count {
                        let subrect = self.read_color_subrect().await?;

                        self.fill_subrect(tile_rect, &subrect.get_rect(), subrect.pixel);
                    }
                }
                else {
                    for _ in 0..subrect_count {
                        let subrect = self.read_subrect().await?;

                        self.fill_subrect(tile_rect, &subrect.get_rect(), self.foreground);
                    }
                }
            }
        }

        Ok(())
    }

    fn fill_subrect(&mut self, tile_rect: &Rect, subrect: &Rect, pixel: DevicePixel) {
        let bytes_per_pixel = Screen::bytes_per_pixel();
        let top_offset = (tile_rect.location.y + subrect.location.y) as usize * self.fst.screen.bytes_per_row() + 
            (tile_rect.location.x + subrect.location.x) as usize * bytes_per_pixel;

        for y in 0..subrect.size.height {
            let mut offset = top_offset + (y as usize) * self.fst.screen.bytes_per_row();

            for _ in 0..subrect.size.width { 
                self.fst.screen.set_at_offset(offset, pixel);
                offset += bytes_per_pixel;
            }
        }
    }

    async fn read_color_subrect(&mut self) -> Result<ColorSubrect, RfbSessionError> {
        let bytes_per_server_pixel = self.fst.bytes_per_server_pixel();
        let mut buffer: Vec<u8> = vec![0; 2 + bytes_per_server_pixel];

        self.fst.read(&mut buffer[..]).await?;

        Ok(ColorSubrect {
            pixel: self.fst.to_device_pixel(&buffer[0..]),
            xy: buffer[bytes_per_server_pixel],
            wh: buffer[bytes_per_server_pixel+1],
        })
    }

    async fn read_subrect(&mut self) -> Result<Subrect, RfbSessionError> {
        let mut buffer: [u8; 2] = [0; 2];

        self.fst.read(&mut buffer[..]).await?;
        Ok(Subrect{
            xy: buffer[0],
            wh: buffer[1],
        })
    }
}