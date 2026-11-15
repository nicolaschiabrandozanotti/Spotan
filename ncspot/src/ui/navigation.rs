//! Top-level navigation with shared drawing/hit-test geometry.
use unicode_width::UnicodeWidthStr;

pub struct Item {
    pub id: &'static str,
    pub label: &'static str,
    pub x: usize,
    pub width: usize,
}

pub fn height(height: usize) -> usize {if height>=9 {3} else {1}}

pub fn items(width: usize) -> Vec<Item> {
    let compact=width<30;
    let labels=if compact {["Bib","Bus","Col"]} else {["Biblioteca","Buscar","Cola"]};
    let padding=if compact {2} else {4};
    let total=labels.iter().map(|label|label.width()+padding).sum::<usize>()+2;
    let mut x=width.saturating_sub(total)/2;
    let mut result=Vec::new();
    for (id,label) in ["library","search","queue"].into_iter().zip(labels) {
        let available=width.saturating_sub(x);
        if available==0 {break;}
        let item_width=(label.width()+padding).min(available);
        result.push(Item {id,label,x,width:item_width});x+=item_width+1;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn navigation_has_separate_click_targets_at_narrow_and_wide_sizes() {
        for width in [0,1,17,20,30,60,100] {
            let items=items(width);
            for item in &items {assert!(item.x+item.width<=width);}
            for pair in items.windows(2) {assert!(pair[0].x+pair[0].width<pair[1].x);}
            if width>=17 {assert_eq!(items.len(),3);}
        }
    }
}
