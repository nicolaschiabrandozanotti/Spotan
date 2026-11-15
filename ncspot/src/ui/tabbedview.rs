use std::cmp::min;

use cursive::{
    Cursive, Printer, Vec2, View,
    align::HAlign,
    event::{Event, EventResult, MouseButton, MouseEvent},
    theme::{ColorStyle,Effect},
    view::Nameable,
    views::NamedView,
};
use unicode_width::UnicodeWidthStr;

use crate::{
    command::{Command, MoveAmount, MoveMode},
    commands::CommandResult,
    traits::{BoxedViewExt, IntoBoxedViewExt, ViewExt},
};

/// A view that displays other views in a tab layout.
#[derive(Default)]
pub struct TabbedView {
    /// The list of tabs
    tabs: Vec<NamedView<BoxedViewExt>>,
    /// The index of the currently visible tab from `tabs`
    selected: usize,
    /// The size given to the last call to `layout()`
    last_layout_size: Vec2,
}

impl TabbedView {
    pub fn new() -> Self {
        Default::default()
    }

    /// Add `view` as a new tab to the end of this [TabbedView].
    pub fn add_tab(&mut self, title: impl Into<String>, view: impl IntoBoxedViewExt) {
        let tab = BoxedViewExt::new(view.into_boxed_view_ext()).with_name(title);
        self.tabs.push(tab);
    }

    /// Return a mutable reference to the tab at `index`, or None if there is no tab at `index`.
    pub fn tab_mut(&mut self, index: usize) -> Option<&mut NamedView<BoxedViewExt>> {
        self.tabs.get_mut(index)
    }

    /// Return a mutable reference to the selected tab, or None if there is no selected tab
    /// currently.
    pub fn selected_tab_mut(&mut self) -> Option<&mut NamedView<BoxedViewExt>> {
        self.tab_mut(self.selected)
    }

    /// Return the amount of tabs in this view.
    pub fn len(&self) -> usize {
        self.tabs.len()
    }

    /// Check whether there are tabs in this [TabbedView].
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Set the tab at `index` as currently visible.
    pub fn set_selected(&mut self, index: usize) {
        self.selected = min(self.len().saturating_sub(1), index);
    }

    /// Move the focus by `amount`, clipping at the edges.
    pub fn move_selected(&mut self, amount: isize) {
        self.selected = min(
            self.selected.saturating_add_signed(amount),
            self.len().saturating_sub(1),
        );
    }

    pub fn move_left(&mut self) {
        self.move_selected(-1);
    }

    pub fn move_right(&mut self) {
        self.move_selected(1);
    }

    /// Move the focus to the first tab.
    pub fn select_first(&mut self) {
        self.selected = 0;
    }

    /// Move the focus to the last tab.
    pub fn select_last(&mut self) {
        self.selected = self.len().saturating_sub(1);
    }

    /// Return whether we are on the first tab.
    pub fn on_first_tab(&mut self) -> bool {
        self.selected == 0
    }

    /// Return whether we are on the last tab.
    pub fn on_last_tab(&mut self) -> bool {
        self.selected == self.len().saturating_sub(1)
    }

    pub fn selected_title(&self) -> &str {
        self.tabs.get(self.selected).map(|tab|display_name(tab.name())).unwrap_or("")
    }

    fn header(&self) -> Header {
        header_layout(&self.tabs.iter().map(|tab|display_name(tab.name())).collect::<Vec<_>>(),self.selected,self.last_layout_size.x)
    }

}

impl View for TabbedView {
    fn draw(&self, printer: &Printer<'_, '_>) {
        if self.is_empty() {
            return;
        }

        let header=self.header();
        printer.with_color(ColorStyle::primary(),|printer| {
            for y in 0..2.min(printer.size.y) {printer.print_hline((0,y),printer.size.x," ");}
        });
        for slot in &header.tabs {
            let selected=self.selected==slot.index;
            let style=if selected {ColorStyle::highlight()} else {ColorStyle::secondary()};
            let cell=printer.offset((slot.x,0)).cropped((slot.width,1));
            cell.with_color(style,|printer| {
                printer.print_hline((0,0),slot.width," ");
                let text=display_name(self.tabs[slot.index].name());
                let offset=HAlign::Center.get_offset(text.width(),slot.width);
                if selected {printer.with_effect(Effect::Bold,|printer|printer.print((offset,0),text));}
                else {printer.print((offset,0),text);}
            });
        }
        printer.with_color(ColorStyle::title_primary(),|printer| {
            if header.before {printer.print((0,0),"‹");}
            if header.after && printer.size.x>0 {printer.print((printer.size.x-1,0),"›");}
        });

        if let Some(tab) = self.tabs.get(self.selected) {
            let printer = printer
                .offset((0, 2.min(printer.size.y)))
                .cropped((printer.size.x, printer.size.y.saturating_sub(2)));

            tab.draw(&printer);
        }
    }

    fn layout(&mut self, size: Vec2) {
        self.last_layout_size = size;
        if let Some(tab) = self.tab_mut(self.selected) {
            tab.layout((size.x, size.y.saturating_sub(2)).into())
        }
    }

    fn on_event(&mut self, event: Event) -> EventResult {
        if let Event::Mouse {
            offset,
            position,
            event,
        } = event
        {
            let position = position.checked_sub(offset);
            if position.is_some_and(|p|p.y<2) {
                if position.is_some_and(|p|p.y==1) {return EventResult::consumed();}
                match event {
                    MouseEvent::WheelUp => self.move_left(),
                    MouseEvent::WheelDown => self.move_right(),
                    MouseEvent::Press(MouseButton::Left) => {
                        if let Some(position)=position {
                            let header=self.header();
                            if header.before && position.x==0 {self.move_left();}
                            else if header.after && position.x+1==self.last_layout_size.x {self.move_right();}
                            else if let Some(slot)=header.tabs.iter().find(|slot|position.x>=slot.x && position.x<slot.x+slot.width) {
                                self.set_selected(slot.index);
                            }
                        }
                    }
                    _ => {}
                };
                return EventResult::consumed();
            }
        }

        if let Some(tab) = self.tab_mut(self.selected) {
            tab.on_event(event.relativized((0, 2)))
        } else {
            EventResult::Ignored
        }
    }
}

impl ViewExt for TabbedView {
    fn on_command(&mut self, s: &mut Cursive, cmd: &Command) -> Result<CommandResult, String> {
        match cmd {
            Command::Move(mode, amount) if matches!(mode, MoveMode::Left | MoveMode::Right) => {
                if matches!(mode, MoveMode::Left) && !self.on_first_tab() {
                    match amount {
                        MoveAmount::Extreme => self.select_first(),
                        MoveAmount::Integer(amount) => self.move_selected(-(*amount) as isize),
                        _ => (),
                    }
                } else if matches!(mode, MoveMode::Right) && !self.on_last_tab() {
                    match amount {
                        MoveAmount::Extreme => self.select_last(),
                        MoveAmount::Integer(amount) => self.move_selected(*amount as isize),
                        _ => (),
                    }
                }
                Ok(CommandResult::Consumed(None))
            }
            _ => {
                if let Some(tab) = self.selected_tab_mut() {
                    tab.on_command(s, cmd)
                } else {
                    Ok(CommandResult::Ignored)
                }
            }
        }
    }
}


fn display_name(name: &str) -> &str {
    match name {"Tracks"=>"Temas","Albums"=>"Álbumes","Artists"=>"Artistas","Playlists"=>"Listas","Podcasts"=>"Podcasts","Browse"=>"Explorar",_=>name}
}

struct Slot {index:usize,x:usize,width:usize}
struct Header {tabs:Vec<Slot>,before:bool,after:bool}

fn header_layout(labels:&[&str],selected:usize,width:usize) -> Header {
    if labels.is_empty() || width==0 {return Header {tabs:Vec::new(),before:false,after:false};}
    let total=labels.iter().map(|label|label.width()+2).sum::<usize>();
    let gap=usize::from(total+labels.len().saturating_sub(1)<=width);
    if total+(labels.len()-1)*gap<=width {
        let mut x=(width-total-(labels.len()-1)*gap)/2;
        let tabs=labels.iter().enumerate().map(|(index,label)| {let slot=Slot {index,x,width:label.width()+2};x+=slot.width+gap;slot}).collect();
        return Header {tabs,before:false,after:false};
    }
    let available=width.saturating_sub(4).max(1);
    let selected=selected.min(labels.len()-1);
    let mut first=0;
    while first<selected && labels[first..=selected].iter().map(|label|label.width()+3).sum::<usize>()>available {first+=1;}
    let mut x=if width>=5 {2} else {0};
    let end=width.saturating_sub(usize::from(width>=5)*2);
    let mut tabs=Vec::new();
    for (index,label) in labels.iter().enumerate().skip(first) {
        let desired=label.width()+2;
        if !tabs.is_empty() && x+desired>end {break;}
        if x>=end {break;}
        let slot=Slot {index,x,width:desired.min(end-x)};x+=slot.width+1;tabs.push(slot);
    }
    let after=tabs.last().is_some_and(|slot|slot.index+1<labels.len());
    Header {tabs,before:first>0,after}
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tabs_have_padding_and_keep_the_active_tab_visible() {
        let labels=["Temas","Álbumes","Artistas","Listas","Podcasts","Explorar"];
        for width in [1,5,12,20,30,54,60,80] {for selected in 0..labels.len() {
            let header=header_layout(&labels,selected,width);
            for slot in &header.tabs {assert!(slot.x+slot.width<=width);}
            for slots in header.tabs.windows(2) {assert!(slots[0].x+slots[0].width<=slots[1].x);}
            if width>=12 {assert!(header.tabs.iter().any(|slot|slot.index==selected));}
            if width>=60 {assert_eq!(header.tabs.len(),labels.len());}
        }}
    }
}
