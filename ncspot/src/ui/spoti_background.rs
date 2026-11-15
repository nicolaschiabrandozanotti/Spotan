//! Draw the shared backdrop only in empty cells after ncspot has drawn its UI.
use cursive::{Printer, Vec2};
use cursive::theme::{Color, ColorStyle, PaletteColor};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

#[path = "../../../src/braille.rs"]
mod braille;

pub struct Background {
    pub enabled: bool,
    raster: Mutex<braille::Raster>,
    last_cells: Mutex<Vec<(usize,usize,u8)>>,
}

impl Default for Background {
    fn default() -> Self {Self {enabled:true,raster:Mutex::new(braille::Raster::default()),last_cells:Mutex::new(Vec::new())}}
}

fn blend(background: Color, accent: Color, amount: f64) -> Color {
    let rgb=|color,fallback|match color {Color::Rgb(r,g,b)=>(r,g,b),_=>fallback};
    let (r,g,b)=rgb(background,(30,30,46));let (ar,ag,ab)=rgb(accent,(203,166,247));
    let channel=|base:u8,target:u8|(base as f64+(target as f64-base as f64)*amount).round() as u8;
    Color::Rgb(channel(r,ar),channel(g,ag),channel(b,ab))
}

/// Leave a cell of padding on either side of text, and avoid selected rows.
fn eligible(blank: bool, left_blank: bool, right_blank: bool, background: Color, base: Color) -> bool {
    blank && left_blank && right_blank && background==base
}

impl Background {
    pub fn clear(&self,printer:&Printer<'_, '_>) {
        let background=printer.theme.palette[PaletteColor::View];
        printer.with_color(ColorStyle::new(background,background),|printer| {
            for (x,y,_) in self.last_cells.lock().unwrap().drain(..) {printer.print((x,y)," ");}
        });
    }

    pub fn draw(&self,printer:&Printer<'_, '_>,top:usize,bottom:usize) {
        if !self.enabled || printer.size.x==0 || bottom<=top {return;}
        let width=printer.size.x;let height=bottom-top;
        let now=SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs_f64();
        let mut raster=self.raster.lock().unwrap();
        let dots=raster.render(width,height,now);
        let background=printer.theme.palette[PaletteColor::View];
        let violet=blend(background,printer.theme.palette[PaletteColor::TitlePrimary],0.32);
        let green=blend(background,printer.theme.palette.custom("playing").copied().unwrap_or(violet),0.27);
        // Release Cursive's buffer lock before calling Printer::print.
        let mut cells=self.last_cells.lock().unwrap();
        cells.clear();
        printer.on_window(|window| {
            let blank=|window:&cursive::buffer::Window<'_>,x:usize,y:usize| {
                window.cell_at(Vec2::new(x,y)).is_some_and(|cell|cell.text()==" ")
            };
            for y in top..bottom.min(printer.output_size.y) {
                for x in 0..width.min(printer.output_size.x) {
                    let bits=dots[(y-top)*width+x];if bits==0 {continue;}
                    let is_blank=blank(window,x,y);
                    let left=x==0 || blank(window,x-1,y);
                    let right=x+1==width || blank(window,x+1,y);
                    let back=window.style_at_mut((x,y)).map(|style|style.color.back);
                    if back.is_some_and(|back|eligible(is_blank,left,right,back,background)) {cells.push((x,y,bits));}
                }
            }
        });
        for &(x,y,bits) in cells.iter() {
            let color=if (x+(y-top)*2)%19<12 {violet}else{green};
            let character=char::from_u32(0x2800+bits as u32).unwrap();
            printer.with_color(ColorStyle::new(color,background),|printer|printer.print((x,y),character.encode_utf8(&mut [0;4])));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_text_padding_and_selected_rows() {
        let base=Color::Rgb(30,30,46);let selected=Color::Rgb(69,71,90);
        assert!(eligible(true,true,true,base,base));
        assert!(!eligible(false,true,true,base,base));
        assert!(!eligible(true,false,true,base,base));
        assert!(!eligible(true,true,false,base,base));
        assert!(!eligible(true,true,true,selected,base));
    }
}
