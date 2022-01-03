use celestium::transaction::BASE_TRANSACTION_MESSAGE_LEN;
/* Maniuplating a 1000x1000 pixel canvas */
use sha3::{Digest, Sha3_224};

pub(crate) const PIXEL_HASH_SIZE: usize = 28;
const NUM_COLORS: u8 = 16;
const WIDTH: usize = 1000;
const HEIGHT: usize = 1000;

pub type Color = u8;

#[derive(Clone)]
pub struct Pixel {
    back_hash: [u8; PIXEL_HASH_SIZE],
    pub color: Color,
}

impl Pixel {
    pub fn new(back_hash: [u8; PIXEL_HASH_SIZE], color: Color) -> Pixel {
        Pixel { back_hash, color }
    }

    // pub fn new_rand(rng: &mut rand::rngs::ThreadRng) -> Pixel {
    //     Pixel {
    //         back_hash: [0u8; PIXEL_HASH_SIZE],
    //         color: rng.gen_range(0..3),
    //     }
    // }

    pub fn hash(self, x: u16, y: u16) -> [u8; PIXEL_HASH_SIZE] {
        let mut to_digest = [0u8; 33];
        to_digest[..PIXEL_HASH_SIZE].copy_from_slice(&self.back_hash);
        to_digest[PIXEL_HASH_SIZE] = (x >> 8) as u8;
        to_digest[PIXEL_HASH_SIZE + 1] = (x & 0xff) as u8;
        to_digest[PIXEL_HASH_SIZE + 2] = (y >> 8) as u8;
        to_digest[PIXEL_HASH_SIZE + 3] = (y & 0xff) as u8;
        to_digest[PIXEL_HASH_SIZE + 4] = self.color as u8;
        let mut hash = [0u8; PIXEL_HASH_SIZE];
        hash.copy_from_slice(&Sha3_224::digest(&to_digest));
        hash
    }
}

#[derive(Clone)]
pub struct Canvas {
    canvas: Vec<Vec<Pixel>>,
}

impl Canvas {
    pub fn new_test() -> Canvas {
        //let mut rng = rand::thread_rng();
        let mut canvas = Vec::new();
        for _ in 0..HEIGHT {
            let mut row = Vec::new();
            for _ in 0..WIDTH {
                row.push(Pixel::new([0u8; PIXEL_HASH_SIZE], 7));
                //row.push(Pixel::new_rand(&mut rng));
            }
            canvas.push(row);
        }
        Canvas { canvas }
    }

    pub fn serialize_colors(&self) -> Vec<u8> {
        // serialize entire canvas into an array of only colors
        self.canvas
            .iter()
            .map(|column| column.iter().map(|p| p.color))
            .flatten()
            .collect()
    }

    pub fn get_pixel(&self, x: usize, y: usize) -> Result<Pixel, String> {
        // TODO: bounds check?
        if x < WIDTH && y < HEIGHT {
            match self.canvas.get(x) {
                Some(row) => match row.get(y) {
                    Some(p) => Ok(p.clone()),
                    None => Err(format!("Could not find row {}", y)),
                },
                None => Err(format!("Could not find column {}", x)),
            }
        } else {
            Err(format!(
                "Index ({}, {}) out of bounds ({}, {})",
                x, y, WIDTH, HEIGHT
            ))
        }
    }

    pub fn set_pixel(&mut self, x: usize, y: usize, p: Pixel) -> Result<(), String> {
        match (x < WIDTH, y < HEIGHT) {
            (true, true) => {
                self.canvas[x as usize][y as usize] = p;
                Ok(())
            }
            (false, _) => Err(format!("x should be less than {}", WIDTH)),
            (_, false) => Err(format!("y should be less than {}", HEIGHT)),
        }
    }

    pub fn parse_pixel(
        bytes: [u8; BASE_TRANSACTION_MESSAGE_LEN],
    ) -> Result<(usize, usize, Pixel), String> {
        // parse the pixel data from a pixel-NFT transaction
        let mut hash: [u8; PIXEL_HASH_SIZE] = [0u8; PIXEL_HASH_SIZE];
        hash.copy_from_slice(&bytes[..PIXEL_HASH_SIZE]);
        let x: usize =
            ((bytes[PIXEL_HASH_SIZE] as usize) << 8) + (bytes[PIXEL_HASH_SIZE + 1] as usize);
        let y: usize =
            ((bytes[PIXEL_HASH_SIZE + 2] as usize) << 8) + (bytes[PIXEL_HASH_SIZE + 3] as usize);
        let color: u8 = bytes[PIXEL_HASH_SIZE + 4];
        match (x < WIDTH, y < HEIGHT, color < NUM_COLORS) {
            (true, true, true) => Ok((x, y, Pixel::new(hash, color))),
            (false, _, _) => Err(format!("x should be less than {}", WIDTH)),
            (_, false, _) => Err(format!("y should be less than {}", HEIGHT)),
            (_, _, false) => Err(format!("color should be less than {}", NUM_COLORS)),
        }
    }
}
