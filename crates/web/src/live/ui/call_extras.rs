//! Call extras in the call screen and channel settings: the Soundboard, More
//! and Save Clip buttons, and a voice channel's bitrate. The panels themselves
//! are built by `public/callextras.js` and the modules it loads on demand.
use super::*;

/// Buttons beside the call controls. On a phone the Soundboard lives in More.
pub(super) fn controls(a: &App) -> AnyView {
    if a.voice_channel.is_none() {
        return ().into_any();
    }
    view! {
        <span class="call-clip-slot contents" hidden=true>{ib("clapperboard","Save Clip","media:clip")}</span>
        <span class="contents phone:hidden">{ib("audio-lines","Soundboard","media:soundboard")}</span>
        {ib("more-horizontal","More Options","media:more")}
    }
    .into_any()
}

/// A voice channel's Opus bitrate, filled in and saved by `callextras.js`.
pub(super) fn bitrate_section(a: &App, c: &ChannelSummary) -> AnyView {
    if c.kind != ChannelKind::Voice {
        return ().into_any();
    }
    let manage = settings::can(a, Permissions::MANAGE_CHANNELS);
    view! {<section class="voice-bitrate-settings mt-6" data-voice-bitrate=c.id.0.to_string() data-can-manage=manage.to_string() aria-label="Voice Quality"><p class="muted-copy">"Loading voice quality…"</p></section>}
    .into_any()
}
