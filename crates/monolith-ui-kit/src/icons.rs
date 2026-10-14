//! The icons: glyphs of the Lucide icon font cut down to these
//! (`assets/README.md`). One style everywhere; an icon supplements a label
//! where the label alone could be misread, and stands alone only where
//! its tooltip and its accessible name say what it does.

use egui::{Align2, Color32, FontFamily, FontId, Painter, Pos2, Rect};

use crate::fonts;

/// An icon: a code point of the icon font.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Icon(char);

impl Icon {
    /// The code point.
    pub const fn char(self) -> char {
        self.0
    }

    /// The font of the icon at `size` points.
    pub fn font(size: f32) -> FontId {
        FontId::new(size, FontFamily::Name(fonts::ICONS.into()))
    }

    /// Paints the icon centred on `center`. Returns the rectangle it took.
    pub fn paint(self, painter: &Painter, center: Pos2, size: f32, color: Color32) -> Rect {
        painter.text(
            center,
            Align2::CENTER_CENTER,
            self.0,
            Self::font(size),
            color,
        )
    }
}

/// Lucide `send`.
pub const SEND: Icon = Icon('\u{E152}');
/// Lucide `plus`.
pub const PLUS: Icon = Icon('\u{E13D}');
/// Lucide `user`.
pub const USER: Icon = Icon('\u{E19F}');
/// Lucide `users`.
pub const USERS: Icon = Icon('\u{E1A4}');
/// Lucide `user-plus`.
pub const USER_PLUS: Icon = Icon('\u{E1A2}');
/// Lucide `user-check`.
pub const USER_CHECK: Icon = Icon('\u{E1A0}');
/// Lucide `user-x`.
pub const USER_X: Icon = Icon('\u{E1A3}');
/// Lucide `ticket`.
pub const TICKET: Icon = Icon('\u{E20F}');
/// Lucide `qr-code`.
pub const QR_CODE: Icon = Icon('\u{E1DF}');
/// Lucide `copy`.
pub const COPY: Icon = Icon('\u{E09E}');
/// Lucide `ban`.
pub const BAN: Icon = Icon('\u{E051}');
/// Lucide `trash-2`.
pub const TRASH_2: Icon = Icon('\u{E18E}');
/// Lucide `settings`.
pub const SETTINGS: Icon = Icon('\u{E154}');
/// Lucide `refresh-cw`.
pub const REFRESH_CW: Icon = Icon('\u{E145}');
/// Lucide `chevron-left`.
pub const CHEVRON_LEFT: Icon = Icon('\u{E06E}');
/// Lucide `chevron-right`.
pub const CHEVRON_RIGHT: Icon = Icon('\u{E06F}');
/// Lucide `chevron-down`.
pub const CHEVRON_DOWN: Icon = Icon('\u{E06D}');
/// Lucide `chevron-up`.
pub const CHEVRON_UP: Icon = Icon('\u{E070}');
/// Lucide `ellipsis-vertical`.
pub const ELLIPSIS_VERTICAL: Icon = Icon('\u{E0B7}');
/// Lucide `check`.
pub const CHECK: Icon = Icon('\u{E06C}');
/// Lucide `check-check`.
pub const CHECK_CHECK: Icon = Icon('\u{E38E}');
/// Lucide `clock`.
pub const CLOCK: Icon = Icon('\u{E087}');
/// Lucide `x`.
pub const X: Icon = Icon('\u{E1B2}');
/// Lucide `search`.
pub const SEARCH: Icon = Icon('\u{E151}');
/// Lucide `triangle-alert`.
pub const TRIANGLE_ALERT: Icon = Icon('\u{E193}');
/// Lucide `circle-alert`.
pub const CIRCLE_ALERT: Icon = Icon('\u{E077}');
/// Lucide `info`.
pub const INFO: Icon = Icon('\u{E0F9}');
/// Lucide `file`.
pub const FILE: Icon = Icon('\u{E0C0}');
/// Lucide `download`.
pub const DOWNLOAD: Icon = Icon('\u{E0B2}');
/// Lucide `folder-open`.
pub const FOLDER_OPEN: Icon = Icon('\u{E247}');
/// Lucide `clipboard`.
pub const CLIPBOARD: Icon = Icon('\u{E085}');
/// Lucide `lock`.
pub const LOCK: Icon = Icon('\u{E10B}');
/// Lucide `key-round`.
pub const KEY_ROUND: Icon = Icon('\u{E4A3}');
/// Lucide `fingerprint`.
pub const FINGERPRINT: Icon = Icon('\u{E2CB}');
/// Lucide `badge-check`.
pub const BADGE_CHECK: Icon = Icon('\u{E241}');
/// Lucide `eye`.
pub const EYE: Icon = Icon('\u{E0BA}');
/// Lucide `eye-off`.
pub const EYE_OFF: Icon = Icon('\u{E0BB}');
/// Lucide `pencil`.
pub const PENCIL: Icon = Icon('\u{E1F9}');
/// Lucide `log-out`.
pub const LOG_OUT: Icon = Icon('\u{E10E}');
/// Lucide `message-square`.
pub const MESSAGE_SQUARE: Icon = Icon('\u{E117}');
/// Lucide `inbox`.
pub const INBOX: Icon = Icon('\u{E0F7}');
/// Lucide `mail`.
pub const MAIL: Icon = Icon('\u{E10F}');
/// Lucide `cloud-off`.
pub const CLOUD_OFF: Icon = Icon('\u{E08D}');
/// Lucide `wifi-off`.
pub const WIFI_OFF: Icon = Icon('\u{E1AF}');
/// Lucide `loader-circle`.
pub const LOADER_CIRCLE: Icon = Icon('\u{E10A}');
/// Lucide `sun`.
pub const SUN: Icon = Icon('\u{E178}');
/// Lucide `moon`.
pub const MOON: Icon = Icon('\u{E11E}');
/// Lucide `keyboard`.
pub const KEYBOARD: Icon = Icon('\u{E284}');
/// Lucide `plug`.
pub const PLUG: Icon = Icon('\u{E37F}');
/// Lucide `unplug`.
pub const UNPLUG: Icon = Icon('\u{E45D}');
/// Lucide `arrow-down`.
pub const ARROW_DOWN: Icon = Icon('\u{E042}');
/// Lucide `circle-check`.
pub const CIRCLE_CHECK: Icon = Icon('\u{E226}');
/// Lucide `circle-dashed`.
pub const CIRCLE_DASHED: Icon = Icon('\u{E4B0}');
/// Lucide `hourglass`.
pub const HOURGLASS: Icon = Icon('\u{E296}');
/// Lucide `link`.
pub const LINK: Icon = Icon('\u{E102}');
/// Lucide `share-2`.
pub const SHARE_2: Icon = Icon('\u{E156}');
/// Lucide `id-card`.
pub const ID_CARD: Icon = Icon('\u{E617}');
/// Lucide `contact`.
pub const CONTACT: Icon = Icon('\u{E09C}');
/// Lucide `monitor`.
pub const MONITOR: Icon = Icon('\u{E11D}');
/// Lucide `power`.
pub const POWER: Icon = Icon('\u{E140}');
/// Lucide `waypoints`.
pub const WAYPOINTS: Icon = Icon('\u{E542}');
/// Lucide `network`.
pub const NETWORK: Icon = Icon('\u{E125}');
/// Lucide `square-pen`.
pub const SQUARE_PEN: Icon = Icon('\u{E172}');
/// Lucide `shield-off`.
pub const SHIELD_OFF: Icon = Icon('\u{E15A}');
/// Lucide `undo-2`.
pub const UNDO_2: Icon = Icon('\u{E2A1}');
/// Lucide `external-link`.
pub const EXTERNAL_LINK: Icon = Icon('\u{E0B9}');
/// Lucide `file-text`.
pub const FILE_TEXT: Icon = Icon('\u{E0CC}');
/// Lucide `save`.
pub const SAVE: Icon = Icon('\u{E14D}');
