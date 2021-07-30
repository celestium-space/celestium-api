/* Maniuplating a 1000x1000 pixel canvas */
use std::convert::TryInto;

const NUM_COLORS: u8 = 16;
const WIDTH: u16 = 1000;
const HEIGHT: u16 = 1000;
const EMPTY_PIXEL: Pixel = ([0u8; 32], 0u8);

type Color = u8;
pub type Pixel = ([u8; 32], Color);
pub type Canvas = [[Pixel; HEIGHT as usize]; WIDTH as usize];


pub fn init_canvas() -> Canvas {
    // TODO: is this the rust-y way to do this?
    //       I feel like `Canvas::new()` might be more idiomatic
    //       but I don't really know how to do that
    //       ¯\_(ツ)_/¯
    [[EMPTY_PIXEL; HEIGHT as usize]; WIDTH as usize]
}

pub fn get_pixel(canvas: Canvas, x: u16, y: u16) -> Pixel {
    canvas[x as usize][y as usize]
}

pub fn set_pixel(mut canvas: Canvas, x: u16, y: u16, p: Pixel) -> Result<(), String> {
    match (x < WIDTH, y < HEIGHT) {
        (true, true) => {
            canvas[x as usize][y as usize] = p;
            Ok(())
        }
        (false, _) => { Err(format!("x should be less than {}", WIDTH)) }
        (_, false) => { Err(format!("y should be less than {}", HEIGHT)) }
    }
}

pub fn parse_pixel(bytes: [u8; 37]) -> Result<(u16, u16, Pixel), String> {
    // parse the pixel data from a pixel-NFT transaction
    let hash: [u8; 32] = read_hash_slice(&bytes[0..31]);
    let x: u16 = u16::from_be_bytes([bytes[32], bytes[33]]);
    let y: u16 = u16::from_be_bytes([bytes[34], bytes[35]]);
    let color: u8 = bytes[36];
    match (x < WIDTH, y < HEIGHT, color < NUM_COLORS) {
        (true, true, true) => { Ok((x, y, (hash, color))) }
        (false, _, _) => { Err(format!("x should be less than {}", WIDTH)) }
        (_, false, _) => { Err(format!("y should be less than {}", HEIGHT)) }
        (_, _, false) => { Err(format!("color should be less than {}", NUM_COLORS)) }
    }
}

fn read_hash_slice(slice: &[u8]) -> [u8; 32] {
    // try to convert a 32 byte slice into a 32 byte array
    slice.try_into().expect("Unreachable: Slice was incorrect length.")
}
