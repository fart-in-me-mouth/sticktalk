//! Consent is never persisted. Injection is a separate, fail-closed desktop seam.
use sticktalk_proto::VoiceSession;
#[derive(Clone, Copy, Debug)]
pub struct Grant {
    pub sharer: VoiceSession,
    pub controller: VoiceSession,
    pub token: u64,
}
#[derive(Clone, Debug, Default)]
pub struct Consent {
    pub pending: Option<(VoiceSession, u64)>,
    pub grant: Option<Grant>,
    /// Only set by clicking Allow for this pending request, never from network input.
    approved: Option<VoiceSession>,
}
impl Consent {
    pub fn request(&mut self, requester: VoiceSession, now: u64, timeout: u32) {
        if self.grant.is_none() && self.pending.is_none() {
            self.pending = Some((
                requester,
                now.saturating_add(u64::from(timeout.min(30_000))),
            ));
        }
    }
    pub fn decide(&mut self, now: u64, allow: bool) -> Option<(VoiceSession, bool)> {
        let (requester, deadline) = self.pending.take()?;
        let allow = allow && now < deadline;
        self.approved = allow.then_some(requester);
        Some((requester, allow))
    }
    pub fn granted(&mut self, sharer: VoiceSession, controller: VoiceSession, token: u64) {
        if self.grant.is_some_and(|g| g.token != token) {
            // A new grant must follow a new local request/decision, not reuse
            // approval from a prior controlling lifetime.
            self.clear();
            return;
        }
        if token != 0 {
            self.grant = Some(Grant {
                sharer,
                controller,
                token,
            });
        }
    }
    pub fn may_inject(&self, own: VoiceSession, token: u64) -> bool {
        self.grant.is_some_and(|g| {
            g.sharer == own && g.token == token && self.approved == Some(g.controller)
        })
    }
    pub fn clear(&mut self) {
        *self = Self::default();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use sticktalk_proto::Id;
    #[test]
    fn explicit_fresh_consent_and_timeout() {
        let a = VoiceSession {
            user_id: Id(1),
            connection_id: Id(2),
        };
        let b = VoiceSession {
            user_id: Id(3),
            connection_id: Id(4),
        };
        let mut c = Consent::default();
        c.granted(a, b, 9);
        assert!(!c.may_inject(a, 9));
        c.clear();
        c.request(b, 0, 30_000);
        assert_eq!(c.decide(30_000, true), Some((b, false)));
        c.request(b, 40_000, 30_000);
        assert_eq!(c.decide(40_001, true), Some((b, true)));
        c.granted(a, b, 10);
        assert!(c.may_inject(a, 10));
        assert!(!c.may_inject(a, 9));
        c.clear();
        assert!(!c.may_inject(a, 10));
    }
}
