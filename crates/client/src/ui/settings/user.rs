use super::*;
use iced::widget::column;
use iced::Padding;
pub(super) fn content(app: &App, page: UserPage) -> Element<'_, Msg> {
    let p = &app.settings.preferences;
    match page {
        UserPage::Account => account(app),
        UserPage::Profiles => profiles(app),
        UserPage::Devices => devices(app),
        UserPage::Voice => super::voice::view(app),
        UserPage::Appearance => column![
            title("Appearance"),
            section("THEME"),
            choice("Dark", true, A::User(UserPage::Appearance)),
            line(),
            title("Message Display"),
            choice(
                "Cozy",
                p.density == Density::Cozy,
                A::Density(Density::Cozy)
            ),
            choice(
                "Compact",
                p.density == Density::Compact,
                A::Density(Density::Compact)
            ),
            line(),
            number(
                "Chat Font Scaling",
                p.chat_font_scale * 100.,
                80.0..=150.0,
                N::FontScale,
                "%"
            ),
            preview_message(app),
            line(),
            crate::ui::media_settings::view(app),
            line(),
            section("CREDITS"),
            small("Inter by Rasmus Andersson (SIL OFL). JetBrains Mono by JetBrains (SIL OFL). Lucide icons (ISC). Twemoji artwork by Twitter and contributors (CC BY 4.0).")
        ]
        .spacing(t::S24)
        .into(),
        UserPage::Accessibility => column![
            title("Accessibility"),
            number(
                "Saturation",
                p.saturation * 100.,
                0.0..=100.0,
                N::Saturation,
                "%"
            ),
            hint("Adjust the intensity of interface colors."),
            line(),
            toggle(
                "Always underline links",
                "Make links easier to distinguish from other text.",
                p.underline_links,
                A::Toggle(T::UnderlineLinks, !p.underline_links),
                true
            ),
            line(),
            title("Reduced Motion"),
            small("StickTalk does not animate interface transitions.")
        ]
        .spacing(t::S24)
        .into(),
        UserPage::Advanced => column![
            title("Advanced"),
            toggle(
                "Developer Mode",
                "Show Copy ID actions in entity menus.",
                p.developer_mode,
                A::Toggle(T::Developer, !p.developer_mode),
                true
            ),
            line(),
            toggle(
                "Start with Windows",
                "Open StickTalk in the tray when you sign in.",
                p.start_with_windows,
                A::Toggle(T::Startup, !p.start_with_windows),
                true
            )
        ]
        .spacing(t::S24)
        .into(),
        UserPage::Privacy => column![
            title("Privacy & Safety"),
            title("Server Privacy Defaults"),
            toggle(
                "Allow direct messages from server members",
                "",
                p.allow_direct_messages,
                A::Toggle(T::DirectMessages, !p.allow_direct_messages),
                false
            ),
            line(),
            title("Friend Requests"),
            toggle(
                "Allow friend requests",
                "",
                p.friend_requests,
                A::Toggle(T::FriendRequests, !p.friend_requests),
                false
            ),
            small("Privacy policy controls are not available on this server yet."),
            line(),
            title("Blocked Users"),
            small(format!(
                "{} users blocked. Manage blocked users from Home → Blocked.",
                app.state
                    .model
                    .relationships
                    .values()
                    .filter(|r| r.kind == RelationshipKind::Blocked)
                    .count()
            ))
        ]
        .spacing(t::S24)
        .into(),
        UserPage::Notifications => {
            let mut c = column![
                title("Notifications"),
                title("Global Defaults"),
                small(
                    "Defaults for notifications on this device. Server overrides take precedence."
                )
            ]
            .spacing(t::S16);
            for (s, l) in [
                ("All Messages", NotificationLevel::All),
                ("Only @mentions", NotificationLevel::Mentions),
                ("Nothing", NotificationLevel::Nothing),
            ] {
                c = c.push(choice(s, p.notification_level == l, A::Level(l)));
            }
            c.push(line())
                .push(toggle(
                    "Notification Sounds",
                    "",
                    p.notification_sounds,
                    A::Toggle(T::Sounds, !p.notification_sounds),
                    true,
                ))
                .push(small(
                    "Play a short, subtle sound only while StickTalk is unfocused.",
                ))
                .into()
        }
        UserPage::Keybinds => {
            let mut c = column![
                title("Keybinds"),
                small("Select a shortcut, then press your new key combination."),
                line(),
                row![
                    section("ACTION").width(Fill),
                    section("KEYBIND").width(t::SETTINGS_NAV)
                ]
            ]
            .spacing(t::S24);
            for (i, (name, key)) in p.keybinds.iter().enumerate() {
                c = c
                    .push(
                        row![
                            label_text(name).width(Fill),
                            secondary(
                                if app.settings.binding == Some(i) {
                                    "Press a key…"
                                } else {
                                    key
                                },
                                A::Bind(i),
                                true
                            )
                            .width(t::SETTINGS_NAV)
                        ]
                        .spacing(t::S16)
                        .align_y(Center),
                    )
                    .push(line());
            }
            c.into()
        }
    }
}
fn account(app: &App) -> Element<'_, Msg> {
    let Some(me) = &app.state.model.me else {
        return hint("Loading account…").into();
    };
    let mut c = Column::new().spacing(t::S24);

    let mock = app.state.session.as_ref().is_some_and(|s| s.config.mock);
    let email = if mock {
        if app.settings.reveal {
            "river@example.com"
        } else {
            "*****@example.com"
        }
    } else {
        "Not provided"
    };
    let reveal: Element<'_, Msg> = if mock {
        secondary(
            if app.settings.reveal {
                "Hide"
            } else {
                "Reveal"
            },
            A::Reveal,
            true,
        )
        .style(t::link)
        .into()
    } else {
        space().into()
    };
    let panel = container(
        column![
            account_row(
                "DISPLAY NAME",
                me.user.display_name.as_deref().unwrap_or(&me.user.username),
                A::EditAccount(false)
            ),
            account_row("USERNAME", &me.user.username, A::EditAccount(true)),
            row![
                column![section("EMAIL"), small(email).size(st::LABEL)]
                    .spacing(t::S8)
                    .width(Fill),
                reveal
            ]
            .spacing(t::S16)
            .align_y(Center)
        ]
        .spacing(t::S24),
    )
    .padding(t::S16)
    .width(Fill)
    .style(st::account_panel);
    let banner = container(space().width(Fill).height(st::BANNER))
        .style(move |_| st::banner(st::rgb(app.settings.profile.color)));
    let identity = row![
        column![
            title(app.state.model.name(me.user.id)),
            small(&me.user.username)
        ]
        .spacing(t::S8)
        .width(Fill),
        secondary("Edit User Profile", A::User(UserPage::Profiles), true)
    ]
    .spacing(t::S16)
    .align_y(Center);
    let card = column![
        banner,
        container(column![identity, panel].spacing(t::S24)).padding(Padding {
            top: st::LARGE_AVATAR / 2. + t::S16,
            left: t::S16,
            right: t::S16,
            bottom: t::S16
        })
    ];
    let face = container(super::super::avatar_on(
        app,
        me.user.id,
        st::LARGE_AVATAR,
        true,
        t::BG_ELEVATED,
    ))
    .padding(t::S8)
    .style(st::avatar_ring);
    c = c.push(title("My Account")).push(
        container(stack![
            card,
            container(face).padding(Padding {
                top: st::BANNER - st::LARGE_AVATAR / 2. - t::S8,
                left: t::S8,
                ..Padding::ZERO
            })
        ])
        .width(Fill)
        .style(st::account_card),
    );
    if app.settings.edit_account {
        c = c.push(field(
            if app.settings.edit_username {
                "USERNAME"
            } else {
                "DISPLAY NAME"
            },
            "",
            if app.settings.edit_username {
                &app.form.code
            } else {
                &app.settings.profile.display_name
            },
            if app.settings.edit_username {
                F::Username
            } else {
                F::DisplayName
            },
            true,
        ));
    }
    c = c.push(line());

    c = c.push(title("Password & Security")).push(
        row![
            column![
                label_text("Password"),
                small(if app.methods.as_ref().is_some_and(|m| m.has_password) {
                    "A password is set for your account."
                } else {
                    "Add a password to sign in without Google."
                })
            ]
            .spacing(t::S8)
            .width(Fill),
            super::super::action(
                if app.methods.as_ref().is_some_and(|m| m.has_password) {
                    "Change Password"
                } else {
                    "Set Password"
                },
                Msg::Open(Dialog::Password)
            )
            .style(st::secondary)
        ]
        .spacing(t::S24)
        .align_y(Center),
    );

    c = c.push(line());

    c = c.push(title("Sign-in Methods"));
    c = c
        .push(label_text(
            "Confirm your current password to link or unlink Google",
        ))
        .push(
            text_input(
                "Current password (or leave blank to verify with Google)",
                &app.form.name,
            )
            .secure(true)
            .on_input(|v| Msg::Field(0, v))
            .padding(t::S12)
            .size(t::BODY)
            .style(t::input),
        );
    if let Some(m) = &app.methods {
        let mut google = row![column![
            label_text("Google"),
            small(if m.google_linked {
                "Linked"
            } else {
                "Not linked"
            })
        ]
        .spacing(t::S8)
        .width(Fill)]
        .align_y(Center);
        if m.google_linked {
            google = google.push(disabled_tip(
                super::super::action(
                    "Unlink",
                    Msg::Account(crate::backend::AccountAction::Unlink),
                )
                .on_press_maybe(
                    (!app.busy && m.has_password)
                        .then_some(Msg::Account(crate::backend::AccountAction::Unlink)),
                )
                .style(st::secondary),
                m.has_password,
                "Set a password before unlinking Google.",
            ));
        } else if app.google_available() {
            google = google.push(
                super::super::action("Link", Msg::GoogleStart(true))
                    .on_press_maybe((!app.busy).then_some(Msg::GoogleStart(true)))
                    .style(st::secondary),
            );
        }
        c = c.push(google).push(
            row![
                column![
                    label_text("Password"),
                    small(if m.has_password {
                        "Password set"
                    } else {
                        "No password set"
                    })
                ]
                .spacing(t::S8)
                .width(Fill),
                icon(
                    "check",
                    t::ICON,
                    if m.has_password {
                        t::ACCENT
                    } else {
                        t::TEXT_MUTED
                    }
                )
            ]
            .align_y(Center),
        );
        if !m.has_password && m.google_linked {
            c = c.push(hint("Set a password before unlinking Google."));
        }
    } else {
        c = c.push(small("Loading sign-in methods…"));
    }
    if app.oauth_pending() {
        c = c.push(
            row![
                small("Waiting for sign-in…"),
                super::super::action("Cancel", Msg::GoogleCancel).style(t::link)
            ]
            .spacing(t::S16),
        );
    }

    c = c
        .push(line())
        .push(title("Account Status"))
        .push(label_text("Account active"))
        .push(small("You are signed in to StickTalk."))
        .push(line()).push(title("Delete Account"))
        .push(small("Permanently erase messages and attachments. Backups expire after 14 days; your username is held for 30 days."))
        .push(text_input("Type your username to confirm",&app.form.topic).on_input(|v|Msg::Field(1,v)).padding(t::S12).size(t::BODY).style(t::input))
        .push(super::super::action("Delete Account Permanently",Msg::DeleteAccount).on_press_maybe((!app.busy).then_some(Msg::DeleteAccount)).style(t::danger));

    c.into()
}
fn account_row<'a>(label: &str, value: &str, a: A) -> Element<'a, Msg> {
    row![
        column![section(label), small(value).size(st::LABEL)]
            .spacing(t::S8)
            .width(Fill),
        secondary("Edit", a, true).width(t::ATTACHMENT_CARD)
    ]
    .spacing(t::S16)
    .align_y(Center)
    .into()
}
fn profiles(app: &App) -> Element<'_, Msg> {
    let p = &app.settings.profile;
    let form = column![
        title("Profiles"),
        field(
            "DISPLAY NAME",
            "Display name",
            &p.display_name,
            F::DisplayName,
            true
        ),
        section("AVATAR"),
        row![
            super::super::action("Change Avatar", Msg::PickUpload(UploadPurpose::Avatar))
                .style(t::primary),
            super::super::action(
                "Remove",
                Msg::Command(Command::UpdateProfile(ProfilePatch {
                    avatar_id: Patch::Clear,
                    ..Default::default()
                }))
            )
            .style(t::danger_link)
        ]
        .spacing(t::S8),
        line(),
        section("BANNER COLOR"),
        swatches(p.color, true, app.settings.saving.is_empty()),
        field(
            "PRONOUNS",
            "Add your pronouns",
            &p.pronouns,
            F::Pronouns,
            true
        ),
        column![
            section("ABOUT ME"),
            textarea(
                &app.settings.about_editor,
                true,
                app.settings.saving.is_empty()
            ),
            row![
                space().width(Fill),
                hint(format!("{}/190", p.about.chars().count()))
            ]
        ]
        .spacing(t::S8)
    ]
    .spacing(t::S24);
    if st::columns(app.width).1 < st::CONTENT {
        column![form, profile_preview(app)].spacing(t::S24).into()
    } else {
        row![container(form).width(Fill), profile_preview(app)]
            .spacing(t::S24)
            .into()
    }
}

fn profile_preview(app: &App) -> Element<'_, Msg> {
    let me = app.me().unwrap_or(Id(1));
    let p = &app.settings.profile;
    let username = app
        .state
        .model
        .me
        .as_ref()
        .map_or("", |m| m.user.username.as_str());
    let banner = container(space().width(Fill).height(st::BANNER))
        .style(move |_| st::banner(st::rgb(p.color)));
    let avatar_row = container(super::super::avatar_on(
        app,
        me,
        st::LARGE_AVATAR,
        true,
        t::BG_HOVER,
    ))
    .padding(t::S8)
    .style(|_| {
        let mut s = t::surface(t::BG_HOVER);
        s.border.radius = t::ROUND.into();
        s
    });
    let created = app
        .state
        .model
        .settings
        .profiles
        .get(&me)
        .map(|p| p.created_at);
    let details = column![
        title(if p.display_name.is_empty() {
            username
        } else {
            &p.display_name
        }),
        small(if p.pronouns.is_empty() {
            username.to_owned()
        } else {
            format!("{username} · {}", p.pronouns)
        }),
        line(),
        section("ABOUT ME"),
        small(&p.about),
        line(),
        section("MEMBER SINCE"),
        small(created.map_or("—".into(), crate::datetime::date))
    ]
    .spacing(t::S12);
    let base = column![
        banner,
        container(details).padding(Padding {
            top: st::LARGE_AVATAR / 2. + t::S16,
            left: t::S16,
            right: t::S16,
            bottom: t::S16
        })
    ];
    container(stack![
        base,
        container(avatar_row).padding(Padding {
            top: st::BANNER - st::LARGE_AVATAR / 2. - t::S8,
            left: t::S8,
            ..Padding::ZERO
        })
    ])
    .width(st::PROFILE_PREVIEW)
    .style(|_| {
        let mut s = t::surface(t::BG_HOVER);
        s.border = Border {
            color: t::DIVIDER,
            width: t::HAIRLINE,
            radius: t::S8.into(),
        };
        s
    })
    .into()
}
fn devices(app: &App) -> Element<'_, Msg> {
    let mut c = column![
        title("Devices"),
        small("Manage devices signed in to your account."),
        title("Current Device")
    ]
    .spacing(t::S24);
    for current in [true, false] {
        if !current {
            c = c.push(line()).push(title("Other Devices"));
        }
        let sessions: Vec<_> = app
            .state
            .model
            .settings
            .sessions
            .iter()
            .filter(|s| s.current == current)
            .collect();
        if sessions.is_empty() {
            c = c.push(small(if current {
                "Loading devices…"
            } else {
                "No other devices are signed in."
            }));
        }
        for d in sessions {
            c = c.push(
                row![
                    icon("monitor", t::AVATAR, t::TEXT_SECONDARY),
                    column![
                        label_text(&d.device_name),
                        small(format!("{} · {}", d.platform, d.place)),
                        hint(format!(
                            "Last active {}",
                            crate::datetime::label(d.last_active, false)
                        ))
                    ]
                    .spacing(t::S8)
                    .width(Fill),
                    secondary("Log out", A::LogoutDevice(d.id.clone()), true).style(t::danger_link)
                ]
                .spacing(t::S16)
                .align_y(Center),
            );
        }
    }
    c.push(line())
        .push(
            secondary(
                "Log out all other devices",
                A::LogoutOthers,
                app.state.model.settings.sessions.iter().any(|s| !s.current),
            )
            .style(t::danger),
        )
        .into()
}
fn preview_message(app: &App) -> Element<'_, Msg> {
    container(
        row![
            avatar(app, app.me().unwrap_or(Id(1)), t::AVATAR, false),
            column![
                label_text("River Chen"),
                txt("This is how your messages will look.")
                    .size(t::BODY * app.settings.preferences.chat_font_scale)
            ]
            .spacing(t::S4)
        ]
        .spacing(t::S16),
    )
    .padding(t::S16)
    .style(t::elevated)
    .into()
}
