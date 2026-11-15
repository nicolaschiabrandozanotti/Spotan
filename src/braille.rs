//! Shared braille geometry, independent of either terminal UI library.
use std::f64::consts::TAU;

#[derive(Default)]
pub struct Raster {
    dots: Vec<u8>,
    width: usize,
    height: usize,
    last_tick: Option<u64>,
}

impl Raster {
    fn dot(&mut self, x: i32, y: i32) {
        if x < 0 || y < 0 || x >= (self.width * 2) as i32 || y >= (self.height * 4) as i32 { return; }
        const BITS: [[u8; 2]; 4] = [[0, 3], [1, 4], [2, 5], [6, 7]];
        let (x,y)=(x as usize,y as usize);
        self.dots[y / 4 * self.width + x / 2] |= 1 << BITS[y % 4][x % 2];
    }

    pub fn render(&mut self, width: usize, height: usize, seconds: f64) -> &[u8] {
        let seconds=(seconds*8.0).floor()/8.0;
        let tick=seconds.to_bits();
        if self.width==width && self.height==height && self.last_tick==Some(tick) {
            return &self.dots;
        }
        self.last_tick=Some(tick);
        self.width=width; self.height=height;
        self.dots.resize(width*height,0); self.dots.fill(0);
        if width==0 || height==0 { return &self.dots; }
        let (pixel_width,pixel_height)=((width*2) as f64,(height*4) as f64);
        let phase=seconds*0.28;
        for strand in 0..7 {
            let layer=strand as f64-3.0;
            for x in 0..width*2 {
                let u=x as f64/pixel_width;
                let wave=(u*TAU*1.15+phase+layer*0.24).sin()*0.22
                    +(u*TAU*2.1-phase*0.71+layer*0.16).cos()*0.07;
                self.dot(x as i32,(pixel_height*(0.5+wave+layer*0.038)).round() as i32);
            }
        }
        for index in 0..(width*height/55).clamp(5,48) {
            let seed=index as f64+1.0;
            let u=(seed*0.618_034+seconds*0.008).fract();
            let v=(seed*0.414_214-seconds*0.004).rem_euclid(1.0);
            if (seed*2.37+seconds*0.45).sin()>-0.35 {self.dot((u*pixel_width) as i32,(v*pixel_height) as i32);}
        }
        &self.dots
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_dot_mapping_and_quantized_time() {
        let mut raster=Raster {width:1,height:1,dots:vec![0],..Default::default()};
        raster.dot(0,0);raster.dot(1,3);assert_eq!(raster.dots[0],0b1000_0001);
        let first=raster.render(80,25,0.0).to_vec();
        assert_eq!(first,raster.render(80,25,0.1));
        assert_ne!(first,raster.render(80,25,1.0));
        let resized=raster.render(40,50,1.0).to_vec();
        assert_eq!(resized,Raster::default().render(40,50,1.0));
        assert!(raster.render(0,0,1.0).is_empty());
    }
}
