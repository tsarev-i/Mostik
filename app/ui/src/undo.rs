//! Undo and redo of a message field. The field draws its text itself (the markers of the markup
//! are hidden, see [`crate::editor`]), which leaves the browser's own undo with nothing to go back
//! to: Ctrl+Z after a deletion brought nothing back. So the field keeps its own history of what
//! the user did in it. Text the app puts in for the user (a suggestion picked, a line break) is
//! a step as well; text put in from outside (another chat's draft) starts the history afresh,
//! with [`History::reset`].

/// The field's text and where the caret was, in UTF-16 offsets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct State {
    pub text: String,
    pub caret: u32,
}

/// What a change was, for putting runs of typing or deleting together in one step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Typing,
    Deleting,
    /// A paste, a correction, formatting, an emoji, a link: a step of its own.
    Other,
}

/// How many steps back the history goes.
const MAX_STEPS: usize = 100;

/// Typing or deleting this long after the last of it starts a step of its own, in milliseconds.
const STEP_PAUSE_MS: f64 = 1000.0;

/// The undo and redo steps of a field.
#[derive(Debug, Default)]
pub struct History {
    undo: Vec<State>,
    redo: Vec<State>,
    /// The field after the last change the history knows of: a field that shows something else
    /// has been changed for the user since (see [`History::catch_up`]).
    last: Option<State>,
    run: Option<(Kind, f64)>,
}

impl History {
    /// Records a change of the user's from `before` to `after` at `now_ms`.
    pub fn record(&mut self, before: State, after: &State, kind: Kind, now_ms: f64) {
        self.catch_up(&before);
        if before.text == after.text {
            self.last = Some(after.clone());
            return;
        }
        // Typing on where the last typing ended: a caret moved elsewhere starts a new step.
        let goes_on = kind != Kind::Other
            && self.last.as_ref().is_some_and(|l| l.caret == before.caret)
            && self
                .run
                .is_some_and(|(k, at)| k == kind && now_ms - at < STEP_PAUSE_MS && !self.undo.is_empty());
        if !goes_on {
            self.push(before);
        }
        self.redo.clear();
        self.run = (kind != Kind::Other).then_some((kind, now_ms));
        self.last = Some(after.clone());
    }

    /// Forgets everything: the field shows text put in from outside.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// The state to go back to from `current`, if any.
    pub fn undo(&mut self, current: State) -> Option<State> {
        self.catch_up(&current);
        let back = self.undo.pop()?;
        self.redo.push(current);
        self.went_to(&back);
        Some(back)
    }

    /// The state an undo left, to go forward to from `current`, if any.
    pub fn redo(&mut self, current: State) -> Option<State> {
        self.catch_up(&current);
        let forward = self.redo.pop()?;
        self.push(current);
        self.went_to(&forward);
        Some(forward)
    }

    fn went_to(&mut self, state: &State) {
        self.last = Some(state.clone());
        self.run = None;
    }

    fn push(&mut self, state: State) {
        self.undo.push(state);
        if self.undo.len() > MAX_STEPS {
            self.undo.remove(0);
        }
    }

    /// The field shows `shown`, not what the history saw last: the app changed it for the user
    /// meanwhile, and that is a step of its own.
    fn catch_up(&mut self, shown: &State) {
        if let Some(last) = self.last.take_if(|last| last.text != shown.text) {
            self.push(last);
            self.redo.clear();
            self.run = None;
            self.last = Some(shown.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(text: &str) -> State {
        State {
            text: text.into(),
            caret: text.encode_utf16().count() as u32,
        }
    }

    /// Typing a word goes back in one step; a deletion comes back with Ctrl+Z, and goes again
    /// with redo.
    #[test]
    fn a_deletion_comes_back() {
        let mut h = History::default();
        h.record(st(""), &st("h"), Kind::Typing, 0.0);
        h.record(st("h"), &st("hi"), Kind::Typing, 100.0);
        h.record(st("hi"), &st(""), Kind::Deleting, 5000.0);
        assert_eq!(h.undo(st("")), Some(st("hi")));
        assert_eq!(h.undo(st("hi")), Some(st("")));
        assert_eq!(h.undo(st("")), None);
        assert_eq!(h.redo(st("")), Some(st("hi")));
        assert_eq!(h.redo(st("hi")), Some(st("")));
        assert_eq!(h.redo(st("")), None);
    }

    /// A pause, or a change of another kind, starts a new step; a new change drops the redo.
    #[test]
    fn steps() {
        let mut h = History::default();
        h.record(st(""), &st("a"), Kind::Typing, 0.0);
        h.record(st("a"), &st("ab"), Kind::Typing, 2000.0);
        h.record(st("ab"), &st("ab!"), Kind::Other, 2100.0);
        h.record(st("ab!"), &st("ab!c"), Kind::Typing, 2200.0);
        assert_eq!(h.undo(st("ab!c")), Some(st("ab!")));
        assert_eq!(h.undo(st("ab!")), Some(st("ab")));
        h.record(st("ab"), &st("abx"), Kind::Typing, 9000.0);
        assert_eq!(h.redo(st("abx")), None);
        assert_eq!(h.undo(st("abx")), Some(st("ab")));
        assert_eq!(h.undo(st("ab")), Some(st("a")));
    }

    /// Typing elsewhere in the text right after (Home, a click) is a step of its own.
    #[test]
    fn typing_elsewhere_is_a_new_step() {
        let mut h = History::default();
        h.record(st(""), &st("ab"), Kind::Typing, 0.0);
        let home = State {
            text: "ab".into(),
            caret: 0,
        };
        let after = State {
            text: "xab".into(),
            caret: 1,
        };
        h.record(home, &after, Kind::Typing, 200.0);
        assert_eq!(h.undo(after.clone()).map(|s| s.text), Some("ab".to_owned()));
        assert_eq!(h.undo(st("ab")), Some(st("")));
    }

    /// Text put in from outside (the draft of another chat) is not undone into the old one.
    #[test]
    fn text_set_from_outside_starts_afresh() {
        let mut h = History::default();
        h.record(st(""), &st("draft"), Kind::Typing, 0.0);
        h.reset();
        assert_eq!(h.undo(st("other chat")), None);
        h.record(st("other chat"), &st("other chat!"), Kind::Typing, 100.0);
        assert_eq!(h.undo(st("other chat!")), Some(st("other chat")));
        assert_eq!(h.undo(st("other chat")), None);
    }

    /// A change the app made for the user (a picked suggestion) goes back as a step of its own,
    /// and drops what an undo left to redo.
    #[test]
    fn a_change_made_by_the_app_is_a_step() {
        let mut h = History::default();
        h.record(st(""), &st(":sm"), Kind::Typing, 0.0);
        assert_eq!(h.undo(st("😄 ")), Some(st(":sm")));
        assert_eq!(h.redo(st(":sm")), Some(st("😄 ")));
        h.record(st("😄 "), &st("😄 a"), Kind::Typing, 100.0);
        assert_eq!(h.undo(st("😄 a")), Some(st("😄 ")));
        assert_eq!(h.undo(st("😄 ")), Some(st(":sm")));
        assert_eq!(h.undo(st(":sm")), Some(st("")));
        assert_eq!(h.redo(st("x")), None);
    }
}
