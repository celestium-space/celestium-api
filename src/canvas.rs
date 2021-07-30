/* Maniuplating a 1000x1000 pixel canvas */
use std::convert::TryInto;

const NUM_COLORS: u8 = 16;
type Color = u8;
pub type Pixel = ([u8; 32], Color);
pub type Canvas = [[Pixel; 1000]; 1000];

const EMPTY_PIXEL: Pixel = ([0; 32], 0);

pub fn init_canvas() -> Canvas {
    // TODO: is this the rust-y way to do this?
    //       I feel like `Canvas::new()` might be more idiomatic
    //       but I don't really know how to do that
    //       ¯\_(ツ)_/¯
    [[EMPTY_PIXEL; 1000]; 1000]
}

pub fn get_pixel(canvas: Canvas, x: u16, y: u16) -> Pixel {
    canvas[x as usize][y as usize]
}

pub fn parse_pixel(bytes: [u8; 37]) -> Result<(u16, u16, Pixel), String> {
    // parse the pixel data from a pixel-NFT transaction
    let hash: [u8; 32] = read_hash_slice(&bytes[0..31]);
    let x: u16 = u16::from_be_bytes([bytes[32], bytes[33]]);
    let y: u16 = u16::from_be_bytes([bytes[34], bytes[35]]);
    let color: u8 = bytes[36];
    match (x < 1000, y < 1000, color < 16) {
        (true, true, true) => { Ok((x, y, (hash, color))) }
        (false, _, _) => { Err("x should be less than 1000".to_string()) }
        (_, false, _) => { Err("y should be less than 1000".to_string()) }
        (_, _, false) => { Err("color should be less than 16".to_string()) }
    }
}

fn read_hash_slice(slice: &[u8]) -> [u8; 32] {
    // try to convert a 32 byte slice into a 32 byte array
    slice.try_into().expect("Unreachable: Slice was incorrect length.")
}
