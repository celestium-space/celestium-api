/* Maniuplating a 1000x1000 pixel canvas */
use std::convert::TryInto;

const NUM_COLORS: u8 = 16;
type Color = u8;
type Pixel = ([u8; 32], Color);
pub type Canvas = [[Pixel; 1000]; 1000];

pub fn init_canvas() -> Canvas {
    // TODO: is this the rust-y way to do this?
    //       I feel like `Canvas::new()` would be better
    //       but I don't really know how to do that
    //       ¯\_(ツ)_/¯
    [[([0; 32], 0); 1000]; 1000]
}

pub fn get_pixel(canvas: Canvas, x: u16, y: u16) -> Pixel {
    canvas[x as usize][y as usize]
}

fn read_hash_slice(slice: &[u8]) -> [u8; 32] {
    slice.try_into().expect("Unreachable: Slice was incorrect length.")
}

pub fn parse_pixel(bytes: [u8; 37]) -> (u16, u16, Pixel) {
    let hash: [u8; 32] = read_hash_slice(&bytes[0..31]);

    let x: u16 = u16::from_be_bytes([bytes[32], bytes[33]]);
    let y: u16 = u16::from_be_bytes([bytes[34], bytes[35]]);
    let color: u8 = bytes[36];
    (x, y, (hash, color))
}
