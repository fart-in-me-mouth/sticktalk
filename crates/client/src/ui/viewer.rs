//! Full-window image viewer. Originals are fetched only after explicit opening.
use super::*;
use crate::features::Event;
use iced::widget::column;
use iced::{Point, Vector};
#[derive(Default)]
pub struct State {
    pub items: Vec<Attachment>,
    pub external: Option<String>,
    pub index: usize,
    pub handle: Option<image::Handle>,
    pub decoded: Option<std::sync::Arc<crate::media::Decoded>>,
    pub loading: bool,
    pub error: String,
    pub serial: u64,
    pub scale: Option<f32>,
    pub pan: Vector,
    pub more: bool,
    pub forwarding: bool,
    pub task: Option<iced::task::Handle>,
}
impl State {
    pub fn current(&self) -> Option<&Attachment> {
        self.items.get(self.index)
    }
    pub fn close(&mut self) {
        let serial = self.serial.wrapping_add(1);
        *self = Self {
            serial,
            ..Self::default()
        };
    }
    pub fn fit(&self, app: &App) -> f32 {
        self.current().map_or(1., |a| {
            ((app.width * 0.9 - 2. * t::S32) / a.width.unwrap_or(800).max(1) as f32)
                .min(
                    (app.height * 0.9 - t::HEADER * 2. - t::S24)
                        / a.height.unwrap_or(600).max(1) as f32,
                )
                .min(1.)
        })
    }
}
pub async fn load(
    config: crate::backend::Config,
    a: Attachment,
    size: (u32, u32),
    external: Option<String>,
) -> Result<std::sync::Arc<crate::media::Decoded>, String> {
    let bytes = if config.mock {
        sticktalk_client_core::mock::IMAGE.to_vec()
    } else {
        let http = crate::backend::Http::new(&config.server, &config.token)?;
        let request = if let Some(url) = external {
            if !crate::gifs::inline_url(&url) {
                return Err("Unsupported media origin".into());
            }
            http.client.get(url)
        } else {
            http.get(&format!("/files/{}", a.id.0))
        };
        let response = request
            .send()
            .await
            .map_err(|_| "Could not load original image")?;
        if !response.status().is_success() {
            return Err("This image is no longer available".into());
        }
        crate::backend::limited(response, 128 * 1024 * 1024).await?
    };
    tokio::task::spawn_blocking(move || {
        let mut reader = ::image::ImageReader::new(std::io::Cursor::new(bytes))
            .with_guessed_format()
            .map_err(|_| "Invalid image")?;
        let mut limits = ::image::Limits::default();
        limits.max_image_width = Some(16384);
        limits.max_image_height = Some(16384);
        limits.max_alloc = Some(128 * 1024 * 1024);
        reader.limits(limits);
        let image = reader
            .decode()
            .map_err(|_| "Image cannot be decoded safely")?
            .thumbnail(size.0.clamp(1, 7680), size.1.clamp(1, 4320))
            .to_rgba8();
        Ok(std::sync::Arc::new(crate::media::Decoded {
            id: a.id,
            width: image.width(),
            height: image.height(),
            rgba: image.into_raw(),
        }))
    })
    .await
    .map_err(|_| "Image decoder stopped")?
}
pub fn view(app: &App) -> Element<'_, Msg> {
    let s = &app.features.viewer;
    let Some(a) = s.current() else {
        return space().into();
    };
    let url = s.external.clone().unwrap_or_else(|| {
        app.state
            .session
            .as_ref()
            .map(|c| format!("{}/api/v1/files/{}", c.config.server, a.id.0))
            .unwrap_or_default()
    });
    let mut toolbar = row![
        icon_button("minus", "Zoom out", Msg::feature(Event::ViewerZoom(0.8))),
        icon_button("plus", "Zoom in", Msg::feature(Event::ViewerZoom(1.25))),
        button(hint(format!(
            "{}%",
            (s.scale.unwrap_or_else(|| s.fit(app)) * 100.).round()
        )))
        .on_press(Msg::feature(Event::ViewerReset))
        .padding(t::S8)
        .style(t::icon),
        icon_button("forward", "Forward", Msg::feature(Event::ViewerForwardMenu)),
        icon_button("download", "Save image", Msg::feature(Event::ViewerSave)),
        icon_button("external-link", "Open in browser", Msg::Link(url.clone())),
        icon_button("more-horizontal", "More", Msg::feature(Event::ViewerMore)),
        icon_button("x", "Close viewer", Msg::feature(Event::ViewerClose))
    ]
    .spacing(t::S4)
    .align_y(Center);
    if s.loading {
        toolbar = toolbar.push(icon("loader", t::ICON, t::TEXT_MUTED));
    }
    let header = row![
        hint(format!("{} / {}", s.index + 1, s.items.len())),
        space().width(Fill),
        container(toolbar).padding(t::S4).style(t::elevated)
    ]
    .align_y(Center)
    .padding(Padding {
        left: t::RAIL + t::SIDEBAR - t::VIEWER_MARGIN,
        ..Padding::ZERO
    });
    let handle = s.handle.as_ref().or_else(|| app.images.get(a.id));
    let image: Element<'_, Msg> = if let Some(handle) = handle {
        Element::new(Surface {
            handle: handle.clone(),
            scale: s.scale.unwrap_or_else(|| s.fit(app)),
            source: iced::Size::new(
                a.width.unwrap_or(800) as f32,
                a.height.unwrap_or(600) as f32,
            ),
            pan: s.pan,
        })
    } else {
        container(icon("image", t::AVATAR, t::TEXT_MUTED))
            .center_x(Fill)
            .center_y(Fill)
            .into()
    };
    let middle = row![
        icon_button(
            "arrow-left",
            "Previous image",
            Msg::feature(Event::ViewerNext(-1))
        ),
        image,
        icon_button(
            "arrow-right",
            "Next image",
            Msg::feature(Event::ViewerNext(1))
        )
    ]
    .align_y(Center)
    .height(Fill);
    let footer = row![
        hint(&a.filename),
        hint(super::uploads::bytes(a.byte_size)),
        button(hint("Open original"))
            .on_press(Msg::Link(url.clone()))
            .padding(0)
            .style(t::link)
    ]
    .spacing(t::S12)
    .align_y(Center);
    let mut content = column![header, middle, container(footer).center_x(Fill)].spacing(t::S12);
    if !s.error.is_empty() {
        content = content.push(hint(&s.error).color(t::DANGER));
    }
    let mut layers = vec![
        mouse_area(
            container(space())
                .width(Fill)
                .height(Fill)
                .style(|_| t::surface(t::VIEWER_BACKDROP)),
        )
        .on_press(Msg::feature(Event::ViewerClose))
        .into(),
        container(content)
            .padding(t::VIEWER_MARGIN)
            .width(Fill)
            .height(Fill)
            .into(),
    ];
    if s.more {
        layers.push(
            container(opaque(
                container(
                    column![
                        action("Copy Image", Msg::feature(Event::ViewerCopy)),
                        action("Copy Link", Msg::Copy(url))
                    ]
                    .spacing(t::S4),
                )
                .padding(t::S8)
                .style(t::popover),
            ))
            .align_right(Fill)
            .align_top(Fill)
            .padding(t::VIEWER_MARGIN + t::HEADER)
            .into(),
        );
    }
    if s.forwarding {
        let mut channels = column![title("Forward image")].spacing(t::S8);
        for c in app
            .state
            .model
            .channels
            .values()
            .filter(|c| c.kind != ChannelKind::Category && c.kind != ChannelKind::Voice)
            .take(64)
        {
            channels = channels.push(action(
                c.name.clone().unwrap_or_else(|| "Direct message".into()),
                Msg::feature(Event::ViewerForward(c.id)),
            ));
        }
        channels = channels.push(action("Cancel", Msg::feature(Event::ViewerForwardMenu)));
        layers.push(
            container(opaque(
                container(scroller(channels).height(360))
                    .padding(t::S16)
                    .width(t::PICKER_W)
                    .style(t::popover),
            ))
            .center_x(Fill)
            .center_y(Fill)
            .style(t::dim)
            .into(),
        );
    }
    stack(layers).into()
}
struct Surface {
    handle: image::Handle,
    scale: f32,
    source: iced::Size,
    pan: Vector,
}
#[derive(Default)]
struct Drag {
    origin: Option<(Point, Vector)>,
    click: Option<iced::advanced::mouse::Click>,
}
impl iced::advanced::Widget<Msg, iced::Theme, iced::Renderer> for Surface {
    fn size(&self) -> iced::Size<Length> {
        iced::Size::new(Fill, Fill)
    }
    fn tag(&self) -> iced::advanced::widget::tree::Tag {
        iced::advanced::widget::tree::Tag::of::<Drag>()
    }
    fn state(&self) -> iced::advanced::widget::tree::State {
        iced::advanced::widget::tree::State::new(Drag::default())
    }
    fn layout(
        &mut self,
        _: &mut iced::advanced::widget::Tree,
        _: &iced::Renderer,
        limits: &iced::advanced::layout::Limits,
    ) -> iced::advanced::layout::Node {
        iced::advanced::layout::Node::new(limits.max())
    }
    fn update(
        &mut self,
        tree: &mut iced::advanced::widget::Tree,
        event: &iced::Event,
        layout: iced::advanced::Layout<'_>,
        cursor: iced::mouse::Cursor,
        _: &iced::Renderer,
        _: &mut dyn iced::advanced::Clipboard,
        shell: &mut iced::advanced::Shell<'_, Msg>,
        _: &iced::Rectangle,
    ) {
        let b = layout.bounds();
        let state = tree.state.downcast_mut::<Drag>();
        match event {
            iced::Event::Mouse(iced::mouse::Event::WheelScrolled { delta })
                if cursor.is_over(b) =>
            {
                let y = match delta {
                    iced::mouse::ScrollDelta::Lines { y, .. } => *y,
                    iced::mouse::ScrollDelta::Pixels { y, .. } => *y / 40.,
                };
                let scale = (self.scale * 1.15f32.powf(y)).clamp(0.1, 8.);
                let ratio = scale / self.scale;
                let p = cursor.position().unwrap() - b.center();
                let pan = self.pan * ratio - p * (ratio - 1.);
                shell.publish(Msg::feature(Event::ViewerTransform(scale, pan)));
                shell.capture_event();
            }
            iced::Event::Mouse(iced::mouse::Event::ButtonPressed(iced::mouse::Button::Left))
                if cursor.is_over(b) =>
            {
                let p = cursor.position().unwrap();
                let image = iced::Rectangle::with_size(self.source * self.scale);
                let image = iced::Rectangle {
                    x: b.center_x() - image.width / 2. + self.pan.x,
                    y: b.center_y() - image.height / 2. + self.pan.y,
                    ..image
                };
                if !image.contains(p) {
                    shell.publish(Msg::feature(Event::ViewerClose));
                } else {
                    let click = iced::advanced::mouse::Click::new(
                        p,
                        iced::mouse::Button::Left,
                        state.click,
                    );
                    if click.kind() == iced::advanced::mouse::click::Kind::Double {
                        shell.publish(Msg::feature(Event::ViewerActual));
                    }
                    state.click = Some(click);
                    state.origin = Some((p, self.pan));
                }
                shell.capture_event();
            }
            iced::Event::Mouse(iced::mouse::Event::CursorMoved { position }) => {
                if let Some((p, pan)) = state.origin {
                    shell.publish(Msg::feature(Event::ViewerTransform(
                        self.scale,
                        pan + (*position - p),
                    )));
                    shell.capture_event();
                }
            }
            iced::Event::Mouse(iced::mouse::Event::ButtonReleased(iced::mouse::Button::Left)) => {
                state.origin = None;
            }
            _ => {}
        }
    }
    fn draw(
        &self,
        _: &iced::advanced::widget::Tree,
        renderer: &mut iced::Renderer,
        _: &iced::Theme,
        _: &iced::advanced::renderer::Style,
        layout: iced::advanced::Layout<'_>,
        _: iced::mouse::Cursor,
        viewport: &iced::Rectangle,
    ) {
        use iced::advanced::{image::Renderer as _, Renderer as _};
        let b = layout.bounds();
        let size = self.source * self.scale;
        let image_bounds = iced::Rectangle::new(
            b.center() - iced::Vector::new(size.width / 2., size.height / 2.) + self.pan,
            size,
        );
        renderer.with_layer(b, |r| {
            r.draw_image(
                iced::advanced::image::Image {
                    handle: self.handle.clone(),
                    filter_method: iced::advanced::image::FilterMethod::Linear,
                    rotation: iced::Radians(0.),
                    opacity: 1.,
                    snap: true,
                    border_radius: Default::default(),
                },
                image_bounds,
                *viewport,
            )
        });
    }
    fn mouse_interaction(
        &self,
        tree: &iced::advanced::widget::Tree,
        layout: iced::advanced::Layout<'_>,
        cursor: iced::mouse::Cursor,
        _: &iced::Rectangle,
        _: &iced::Renderer,
    ) -> iced::mouse::Interaction {
        if tree.state.downcast_ref::<Drag>().origin.is_some() {
            iced::mouse::Interaction::Grabbing
        } else if cursor.is_over(layout.bounds()) {
            iced::mouse::Interaction::Grab
        } else {
            iced::mouse::Interaction::None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tiny_images_open_at_native_size_and_double_click_toggles_actual_size() {
        let mut app = App::headless("image-viewer", iced::Size::new(1280., 800.));
        app.features.viewer.items[0].width = Some(32);
        app.features.viewer.items[0].height = Some(32);
        let fit = app.features.viewer.fit(&app);
        assert_eq!(fit, 1.);
        assert!(fit * 32. <= app.height * 0.9);
        let _ = crate::features::update(&mut app, Event::ViewerActual);
        assert_eq!(app.features.viewer.scale, Some(1.));
        let _ = crate::features::update(&mut app, Event::ViewerActual);
        assert_eq!(app.features.viewer.scale, None);
    }
}
