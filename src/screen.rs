
use framebuffer::{self, Framebuffer, FramebufferError, KdMode};
use png::Decoder;

pub struct Screen {
    pub fb: Framebuffer,
    pub image: Vec<u8>,
}

#[derive(Debug, Clone, Copy)]
pub struct DevicePixel(u16);

impl DevicePixel {
    pub fn from_rgb(r: u8, g: u8, b:u8) -> DevicePixel {
        DevicePixel(((r as u16 >> 3) << 11) | (g as u16 >> 2) << 5 | (b as u16 >> 3))
    }

    pub fn from_value(v: u16) -> DevicePixel {
        DevicePixel(v)
    }
}

impl Screen {
    pub fn new() -> Result<Screen, FramebufferError> {
        let fb = Framebuffer::new("/dev/fb0")?;
        let image_size = fb.fix_screen_info.line_length * fb.var_screen_info.yres;
        let image = vec![0; image_size as usize];

        Ok(Screen {fb, image, })
    }

    pub fn set_console_to_graphic_mode() -> Result<(), FramebufferError> {
        Framebuffer::set_kd_mode_ex("/dev/console", KdMode::Graphics)?;
        Ok(())
    }

    pub fn set_console_to_text_mode() -> Result<(), FramebufferError> {
        Framebuffer::set_kd_mode_ex("/dev/console", KdMode::Text)?;
        Ok(())
    }

    pub fn xres(&self) -> usize {
        self.fb.var_screen_info.xres as usize
    }

    pub fn yres(&self) -> usize {
        self.fb.var_screen_info.yres as usize
    }

    pub fn bytes_per_pixel() -> usize {
        2
    }

    pub fn bytes_per_row(&self) -> usize {
        self.fb.fix_screen_info.line_length as usize
    }

    pub fn set_at_offset(&mut self, offset: usize, value: DevicePixel) {
        self.image[offset] = (value.0 & 0xff) as u8;
        self.image[offset + 1] = (value.0 >> 8) as u8;
    }
    
    pub fn update(&mut self) {
        self.fb.write_frame(&self.image);
    }

    pub fn display_png_resource(&mut self, png_image: &'static [u8]) {
        let decoder = Decoder::new(png_image);
        let (info, mut decoded_image_reader) = decoder.read_info().expect("Error decoding image");
        
        self.image.fill(0);         // Fill with black
        let mut offset = (self.yres() - (info.height as usize)) / 2 * self.bytes_per_row() + 
            (self.xres() - (info.width as usize)) / 2 * Self::bytes_per_pixel();

        for _ in 0..info.height {
            match decoded_image_reader.next_row().expect("PNG image decoding error") {
                Some(row_buffer) => {
                    let mut png_row_offset = 0;
                    let mut row_offset = offset;
                    for _ in 0..info.width {
                        let pixel = DevicePixel::from_rgb(row_buffer[png_row_offset], row_buffer[png_row_offset+1], row_buffer[png_row_offset+2]);
                        png_row_offset += 3;

                        self.set_at_offset(row_offset, pixel);
                        row_offset += Self::bytes_per_pixel();
                    }
                }
                None => panic!("Missing PNG row")
            }

            offset += self.bytes_per_row();
        }

        self.update();
    }
}
