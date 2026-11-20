//! Lit shop windows at night (`--shop-light 1|2`, stage N1): what a shop
//! is (`ShopKind`, from the kit planner's shop identity), when it is lit
//! (`Hours`), the colour of its light as a night-adapted eye sees it
//! (`colour`) and how bright its window is (`luminance`). Pure functions of
//! the shop's kind and a per-shop key (`shop_key`), so a block is mixed and
//! every run, client and server alike, lights the same windows. eo-render
//! `shop_lights.rs` places the lights, sky_lights.wgsl `lights_shops` decides
//! each frame which are lit from `Hours`' numbers. Stage N2: `vacant` is the
//! one vacancy rule every consumer reads (the lights, the papered glass and
//! dark rooms, the boarded fronts), and `Fit` what fills a shop's room.
//!
//! Reference (sheet 5.10, Castro and 18th at night, Mar 2026, LED era): shop
//! windows and neon carry the street; bright warm shop interiors spill light
//! onto the sidewalk; the pavement is brighter in front of shops. Sheet 3.7
//! has no measured shop-window luminance (the one study found returned 403):
//! the window luminances below are the interior's illuminance (IES practice
//! for the trade, INFERRED) times a mixed interior reflectance and the
//! glazing's transmission, cross-checked against the sheet's lit window
//! (~6 cd/m2) and advertisement (~50 cd/m2) and its storefront signs
//! (40-450 cd/m2): a bar's window reads under an advertisement, a market's
//! over it, none near a sign. Opening hours are typical of San Francisco's
//! neighbourhood commercial streets (INFERRED: no hours table is in the
//! sheet): most shops close by 19:00-21:00, restaurants by 22:00-23:00, bars
//! at 02:00, a few markets, pharmacies and laundromats round the clock.

use crate::hash::{hash01, hash_string, mix32};
use crate::lights::{DARK_SURROUND_F, NIGHT_ADAPTING_CD_M2, SF_LED_CCT_K, degree_of_adaptation, planckian_xy, xyz_to_linear_srgb};

/// What a shop's window shows at night.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ShopKind {
    /// Restaurants, diners, pizza, sushi, noodles, tacos, burgers.
    Restaurant,
    /// Cafes, coffee, delis, bakeries.
    Cafe,
    /// Bars, taverns, lounges, clubs.
    Bar,
    /// Liquor and wine stores.
    Liquor,
    /// Groceries, markets, corner and convenience stores.
    Market,
    /// Laundromats.
    Laundromat,
    /// Pharmacies.
    Pharmacy,
    /// Salons, barbers, nails, tattoo, clinics, dental, optical, dry cleaners.
    Salon,
    /// Shops: books, records, galleries, clothes, shoes, furniture, hardware, flowers...
    Retail,
    /// Gyms, fitness and yoga studios.
    Fitness,
}

impl ShopKind {
    pub const ALL: [ShopKind; 10] = [
        ShopKind::Restaurant,
        ShopKind::Cafe,
        ShopKind::Bar,
        ShopKind::Liquor,
        ShopKind::Market,
        ShopKind::Laundromat,
        ShopKind::Pharmacy,
        ShopKind::Salon,
        ShopKind::Retail,
        ShopKind::Fitness,
    ];

    pub fn name(self) -> &'static str {
        match self {
            ShopKind::Restaurant => "restaurant",
            ShopKind::Cafe => "cafe",
            ShopKind::Bar => "bar",
            ShopKind::Liquor => "liquor",
            ShopKind::Market => "market",
            ShopKind::Laundromat => "laundromat",
            ShopKind::Pharmacy => "pharmacy",
            ShopKind::Salon => "salon",
            ShopKind::Retail => "retail",
            ShopKind::Fitness => "fitness",
        }
    }

    /// Its row of the trade table: (open hour, its jitter either way, closing
    /// hour (past 24: after midnight), its jitter, the share lit round the
    /// clock, the share of the closed ones that keep a light on and that
    /// light's level (a share of the lit window's), the lamps' colour
    /// temperature (K) and its jitter, the lit window's luminance (cd/m2)).
    /// INFERRED (module doc).
    fn row(self) -> Row {
        let r = |open, open_j, close, close_j, all_day, security, sec_level, cct, cct_j, lum| Row { open, open_j, close, close_j, all_day, security, sec_level, cct, cct_j, lum };
        match self {
            // dining 100-300 lux, warm 2700 K lamps; the window shows the pendants and the bar's back too.
            ShopKind::Restaurant => r(11.0, 1.0, 22.0, 1.25, 0.02, 0.25, 0.06, 2700.0, 300.0, 40.0),
            ShopKind::Cafe => r(7.0, 1.0, 18.0, 2.0, 0.0, 0.3, 0.08, 2900.0, 300.0, 45.0),
            // a bar's room is dim (50-100 lux) and warm, its back bar and neon bright; open until 02:00 (California's last call, ABC).
            ShopKind::Bar => r(15.0, 2.0, 26.0, 0.25, 0.0, 0.15, 0.06, 2300.0, 300.0, 16.0),
            ShopKind::Liquor => r(9.0, 1.0, 23.0, 1.0, 0.05, 0.3, 0.1, 4000.0, 700.0, 90.0),
            // corner and convenience stores: bright (1000 lux and more), a neutral to cool white.
            ShopKind::Market => r(7.0, 1.0, 22.0, 1.5, 0.15, 0.3, 0.1, 4300.0, 900.0, 120.0),
            ShopKind::Laundromat => r(6.0, 1.0, 22.0, 1.0, 0.08, 0.2, 0.05, 5000.0, 800.0, 90.0),
            ShopKind::Pharmacy => r(8.0, 1.0, 21.0, 1.0, 0.10, 0.6, 0.2, 4200.0, 600.0, 100.0),
            ShopKind::Salon => r(10.0, 1.0, 19.0, 1.0, 0.0, 0.2, 0.08, 3700.0, 400.0, 55.0),
            // retail sales floors 500-1000 lux; display windows left lit after closing (a share at a fifth of the level).
            ShopKind::Retail => r(10.0, 1.0, 19.5, 2.0, 0.0, 0.5, 0.2, 3500.0, 500.0, 75.0),
            ShopKind::Fitness => r(6.0, 1.0, 22.0, 1.0, 0.10, 0.2, 0.05, 4200.0, 600.0, 70.0),
        }
    }
}

struct Row {
    open: f64,
    open_j: f64,
    close: f64,
    close_j: f64,
    all_day: f64,
    security: f64,
    sec_level: f64,
    cct: f64,
    cct_j: f64,
    lum: f64,
}

/// The share of shopfronts dark at every hour (empty or closed for good;
/// San Francisco's neighbourhood corridors run several percent vacant).
/// INFERRED.
pub const VACANT_SHARE: f64 = 0.06;
/// A closed shop's light (a few fixtures or the display left on): its trade's
/// level (a share of the lit window's) times a factor in this range. INFERRED.
pub const SECURITY_SPREAD: [f64; 2] = [0.6, 1.4];
/// The level of a vacant front's work light, where it has one.
pub const VACANT_LIGHT: f64 = 0.03;
/// The share of vacant fronts that keep that work light on. INFERRED.
pub const VACANT_WORK_SHARE: f64 = 0.3;
/// Opening and closing times fall on the quarter hour.
pub const HOURS_STEP_H: f64 = 0.25;
/// The windows' luminance varies shop to shop by this factor either way.
pub const LUMINANCE_SPREAD: f64 = 1.5;

/// The trade words the kit planner's shop fascias carry (eo-kit
/// shop_ground.rs `TRADE_WORDS`), by kind.
fn kind_of_word(word: &str) -> Option<ShopKind> {
    Some(match word {
        "RESTAURANT" | "DINER" | "PIZZA" | "SUSHI" | "NOODLES" | "TACOS" | "BURGERS" => ShopKind::Restaurant,
        "CAFE" | "COFFEE" | "DELI" | "BAKERY" => ShopKind::Cafe,
        "BAR" | "TAVERN" | "LOUNGE" | "CLUB" => ShopKind::Bar,
        "LIQUOR" | "WINE" | "SPIRITS" => ShopKind::Liquor,
        "GROCERY" | "MARKET" => ShopKind::Market,
        "LAUNDRY" => ShopKind::Laundromat,
        "PHARMACY" => ShopKind::Pharmacy,
        "SALON" | "BARBER" | "NAILS" | "TATTOO" | "CLINIC" | "DENTAL" | "OPTICAL" | "CLEANERS" => ShopKind::Salon,
        "FITNESS" | "GYM" | "YOGA" => ShopKind::Fitness,
        "BOOKS" | "RECORDS" | "MUSIC" | "VIDEO" | "ANTIQUES" | "GALLERY" | "COPIES" | "SUPPLY" | "STUDIO" | "TAILOR" | "SHOES" | "FLORIST" | "FLOWERS" | "FURNITURE" | "RUGS" | "LIGHTING"
        | "PAINT" | "HARDWARE" | "BIKES" | "PHOTO" | "CAMERA" | "PRINTING" => ShopKind::Retail,
        _ => return None,
    })
}

/// A shop's kind from the kit planner's shop identity: its fascia `text`
/// (the trade word after a name, `name|WORD`, or a name that says its trade)
/// first, then its `shop_type` (eo-kit `resolveShopType`: cafe, bakery, bar,
/// grocery, books, cleaners, studio).
pub fn kind_of(shop_type: &str, text: &str) -> ShopKind {
    let upper = text.to_uppercase();
    let last = upper.rsplit('|').next().unwrap_or("").trim();
    if let Some(k) = kind_of_word(last) {
        return k;
    }
    for token in upper.split(|c: char| !c.is_ascii_uppercase()).filter(|t| !t.is_empty()) {
        if let Some(k) = kind_of_word(token) {
            return k;
        }
    }
    match shop_type {
        "cafe" | "bakery" => ShopKind::Cafe,
        "bar" => ShopKind::Bar,
        "grocery" => ShopKind::Market,
        "cleaners" => ShopKind::Salon,
        _ => ShopKind::Retail,
    }
}

/// A shop's key: its building and its tenancy along the front (every bay of
/// one shop lights together).
pub fn shop_key(building: &str, tenancy: u32) -> u32 {
    mix32(hash_string(building) ^ tenancy.wrapping_add(1).wrapping_mul(0x9e3779b1))
}

/// (stage N2) The one vacancy rule: whether the shop of `kind` and `key`
/// stands empty (`VACANT_SHARE` of fronts, per shop, so a strip is mixed).
/// Its light (`hours`: dark at every hour, or a work light), the papered
/// glass and dark room the rooms shader draws (eo-render shop_lights.rs
/// `fronts`, rooms.wgsl) and the boarded fronts the facade clutter puts up
/// (counted vacant too, eo-render `shop_lights`) all read this.
pub fn vacant(kind: ShopKind, key: u32) -> bool {
    hash01(key, kind as u32, 1) < VACANT_SHARE
}

/// Whether a vacant shop keeps a work light on (`VACANT_WORK_SHARE` of them).
pub fn work_light(kind: ShopKind, key: u32) -> bool {
    vacant(kind, key) && hash01(key, kind as u32, 2) < VACANT_WORK_SHARE
}

/// (stage N2) What fills a shop's room behind its glass: the trade's
/// fittings, drawn by the rooms shader (eo-render shop_rooms.wgsl `sr_*`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Fit {
    /// Gondola runs and wall shelving stacked with goods: markets, liquor, pharmacies, hardware, books.
    Shelves,
    /// Clothing racks and display tables.
    Racks,
    /// Tables and chairs, a counter at the back: restaurants, cafes.
    Dining,
    /// A long counter with a back bar of bottles, stools, a few tables.
    Bar,
    /// Chairs facing a mirror wall: salons, barbers, nails, tattoo.
    Salon,
    /// Washers in a row, dryers stacked along a wall.
    Laundry,
    /// White walls, framed pieces, a few large things: galleries, studios, furniture, lighting.
    Gallery,
    /// A reception counter and chairs along the wall: clinics, dental, optical.
    Clinic,
    /// A counter in front of rails of bagged garments: dry cleaners.
    Cleaners,
    /// A mirror wall and machines: gyms, studios.
    Fitness,
}

impl Fit {
    pub const ALL: [Fit; 10] = [Fit::Shelves, Fit::Racks, Fit::Dining, Fit::Bar, Fit::Salon, Fit::Laundry, Fit::Gallery, Fit::Clinic, Fit::Cleaners, Fit::Fitness];

    pub fn name(self) -> &'static str {
        match self {
            Fit::Shelves => "shelves",
            Fit::Racks => "racks",
            Fit::Dining => "dining",
            Fit::Bar => "bar",
            Fit::Salon => "salon",
            Fit::Laundry => "laundry",
            Fit::Gallery => "gallery",
            Fit::Clinic => "clinic",
            Fit::Cleaners => "cleaners",
            Fit::Fitness => "fitness",
        }
    }
}

/// The goods a shelved shop carries (the shader's palette): 0 groceries, 1
/// bottles, 2 pharmacy, 3 hardware and supplies, 4 books and records, 5 flowers.
pub fn goods_of(kind: ShopKind, shop_type: &str, text: &str) -> u32 {
    let words = trade_words(text);
    let has = |w: &[&str]| words.iter().any(|x| w.contains(&x.as_str()));
    match kind {
        ShopKind::Liquor => 1,
        ShopKind::Pharmacy => 2,
        _ if has(&["FLORIST", "FLOWERS"]) => 5,
        _ if has(&["BOOKS", "RECORDS", "MUSIC", "VIDEO"]) || shop_type == "books" => 4,
        _ if has(&["HARDWARE", "PAINT", "SUPPLY", "BIKES", "COPIES", "PRINTING", "CAMERA", "PHOTO"]) => 3,
        _ => 0,
    }
}

fn trade_words(text: &str) -> Vec<String> {
    text.to_uppercase().split(|c: char| !c.is_ascii_uppercase()).filter(|t| !t.is_empty()).map(|t| t.to_string()).collect()
}

/// What fills the room of a shop of `kind` (its fascia `text` and `shop_type`
/// pick among a trade's fittings: a hardware store is shelved, a gallery
/// sparse, a clothes shop racked).
pub fn fit_of(kind: ShopKind, shop_type: &str, text: &str) -> Fit {
    let words = trade_words(text);
    let has = |w: &[&str]| words.iter().any(|x| w.contains(&x.as_str()));
    match kind {
        ShopKind::Restaurant | ShopKind::Cafe => Fit::Dining,
        ShopKind::Bar => Fit::Bar,
        ShopKind::Liquor | ShopKind::Market | ShopKind::Pharmacy => Fit::Shelves,
        ShopKind::Laundromat => Fit::Laundry,
        ShopKind::Fitness => Fit::Fitness,
        ShopKind::Salon if has(&["CLINIC", "DENTAL", "OPTICAL"]) => Fit::Clinic,
        ShopKind::Salon if has(&["CLEANERS"]) || shop_type == "cleaners" => Fit::Cleaners,
        ShopKind::Salon => Fit::Salon,
        ShopKind::Retail if has(&["GALLERY", "STUDIO", "ANTIQUES", "FURNITURE", "RUGS", "LIGHTING"]) || shop_type == "studio" => Fit::Gallery,
        ShopKind::Retail if goods_of(kind, shop_type, text) != 0 => Fit::Shelves,
        ShopKind::Retail => Fit::Racks,
    }
}

/// When a shop is lit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hours {
    /// The hour it opens, [0, 24).
    pub open: f64,
    /// How many hours from then it is lit (24: round the clock; 0: never, a vacant front).
    pub span: f64,
    /// Its level while closed, a share of the lit window's (0: dark).
    pub security: f64,
}

impl Hours {
    /// Its light at `hour` (0..24, any day): 1 open, `security` closed.
    pub fn level_at(&self, hour: f64) -> f64 {
        if self.span >= 24.0 {
            return 1.0;
        }
        let since = (hour - self.open).rem_euclid(24.0);
        if since < self.span { 1.0 } else { self.security }
    }
}

fn quarter(h: f64) -> f64 {
    (h / HOURS_STEP_H).round() * HOURS_STEP_H
}

/// When the shop of `kind` and `key` is lit.
pub fn hours(kind: ShopKind, key: u32) -> Hours {
    let r = kind.row();
    let u = |salt: u32| hash01(key, kind as u32, salt);
    if vacant(kind, key) {
        return Hours { open: 0.0, span: 0.0, security: if work_light(kind, key) { VACANT_LIGHT } else { 0.0 } };
    }
    if u(3) < r.all_day {
        return Hours { open: 0.0, span: 24.0, security: 0.0 };
    }
    let open = quarter(r.open + (2.0 * u(4) - 1.0) * r.open_j).rem_euclid(24.0);
    let close = quarter(r.close + (2.0 * u(5) - 1.0) * r.close_j);
    let span = (close - open).rem_euclid(24.0).max(HOURS_STEP_H);
    let security = if u(6) < r.security { r.sec_level * (SECURITY_SPREAD[0] + (SECURITY_SPREAD[1] - SECURITY_SPREAD[0]) * u(7)) } else { 0.0 };
    Hours { open, span, security }
}

/// The colour temperature of the shop's lamps (K).
pub fn cct(kind: ShopKind, key: u32) -> f64 {
    let r = kind.row();
    r.cct + (2.0 * hash01(key, kind as u32, 11) - 1.0) * r.cct_j
}

/// The colour (linear sRGB, unit luminance) of a source of `cct` kelvin seen
/// by an observer adapted with degree `d` to a white of `adapt` kelvin (CAT02
/// von Kries toward the D65 output; eo-sky `lights::adapted_white` is the case
/// of a source at the adopted white).
pub fn adapted_source(cct: f64, adapt: f64, d: f64) -> [f64; 3] {
    const M: [[f64; 3]; 3] = [[0.7328, 0.4296, -0.1624], [-0.7036, 1.6975, 0.0061], [0.0030, 0.0136, 0.9834]];
    const MI: [[f64; 3]; 3] = [[1.096124, -0.278869, 0.182745], [0.454369, 0.473533, 0.072098], [-0.009628, -0.005698, 1.015326]];
    let mul = |m: &[[f64; 3]; 3], v: [f64; 3]| [0, 1, 2].map(|r| m[r][0] * v[0] + m[r][1] * v[1] + m[r][2] * v[2]);
    let white = |k: f64| {
        let [x, y] = planckian_xy(k);
        [x / y, 1.0, (1.0 - x - y) / y]
    };
    let (ls, lw, lr) = (mul(&M, white(cct)), mul(&M, white(adapt)), mul(&M, [0.95047, 1.0, 1.08883]));
    let lc = [0, 1, 2].map(|i| ls[i] * (d * lr[i] / lw[i] + (1.0 - d)));
    let c = xyz_to_linear_srgb(mul(&MI, lc)).map(|v| v.max(0.0));
    let l = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    c.map(|v| v / l.max(1e-9))
}

/// The shop's light as a night street's observer sees it (linear sRGB, unit
/// luminance): adapted to SF's 3000 K street lamps as `lights::led_street_lamp_color`
/// is, so a 3000 K shop reads as the lamps do, a bar warmer, a market or a
/// laundromat cooler.
pub fn colour(kind: ShopKind, key: u32) -> [f64; 3] {
    adapted_source(cct(kind, key), SF_LED_CCT_K, degree_of_adaptation(NIGHT_ADAPTING_CD_M2, DARK_SURROUND_F))
}

/// The lit window's luminance seen from the street (cd/m2): the trade's,
/// shop to shop within `LUMINANCE_SPREAD` either way.
pub fn luminance(kind: ShopKind, key: u32) -> f64 {
    kind.row().lum * LUMINANCE_SPREAD.powf(2.0 * hash01(key, kind as u32, 21) - 1.0)
}

/// Where a lit window's light falls to `floor_lux` (m): the reach of a point
/// of `candela` (inverse square), the lights' range in the frame list.
pub fn reach_m(candela: f64, floor_lux: f64) -> f64 {
    (candela / floor_lux.max(1e-6)).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lum(c: [f64; 3]) -> f64 {
        0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
    }

    /// The hours rule: most shops dark by 21:00, restaurants and bars lit
    /// late, a few round the clock; a block is mixed, never all lit or all
    /// dark; the same key gives the same hours.
    #[test]
    fn the_hours_rule_mixes_a_block_and_keeps_the_late_trades_lit() {
        let lit_share = |kind: ShopKind, hour: f64| {
            let n = 4000;
            (0..n).filter(|i| hours(kind, mix32(*i as u32 + 7)).level_at(hour) >= 1.0).count() as f64 / n as f64
        };
        let (retail_21, retail_noon) = (lit_share(ShopKind::Retail, 21.0), lit_share(ShopKind::Retail, 13.0));
        let (rest_21, rest_2330) = (lit_share(ShopKind::Restaurant, 21.0), lit_share(ShopKind::Restaurant, 23.5));
        let (bar_2330, bar_1, bar_4) = (lit_share(ShopKind::Bar, 23.5), lit_share(ShopKind::Bar, 1.0), lit_share(ShopKind::Bar, 4.0));
        let (market_3, salon_21, cafe_21) = (lit_share(ShopKind::Market, 3.0), lit_share(ShopKind::Salon, 21.0), lit_share(ShopKind::Cafe, 21.0));
        println!("lit: retail 13 h {retail_noon:.2}, 21 h {retail_21:.2}; restaurants 21 h {rest_21:.2}, 23:30 {rest_2330:.2}; bars 23:30 {bar_2330:.2}, 01 h {bar_1:.2}, 04 h {bar_4:.2}; markets 03 h {market_3:.2}; salons 21 h {salon_21:.2}; cafes 21 h {cafe_21:.2}");
        assert!(retail_noon > 0.9 && retail_21 < 0.25, "shops open by day, most dark after 21:00");
        assert!(salon_21 < 0.1 && cafe_21 < 0.35);
        assert!(rest_21 > 0.75 && rest_2330 < 0.3, "restaurants lit at 21:00, most closed by 23:30");
        assert!(bar_2330 > 0.9 && bar_1 > 0.9 && bar_4 < 0.05, "bars lit until 02:00");
        assert!(market_3 > 0.08 && market_3 < 0.25, "a few markets round the clock: {market_3}");
        // a vacant front is dark at every hour; some closed shops keep a dim light.
        let vacant = (0..4000u32).map(|i| hours(ShopKind::Retail, mix32(i + 7))).filter(|h| h.span == 0.0).count();
        assert!((150..350).contains(&vacant), "vacant fronts {vacant} of 4000");
        let sec: Vec<f64> = (0..4000u32).map(|i| hours(ShopKind::Retail, mix32(i + 7)).level_at(23.0)).filter(|l| *l > 0.0 && *l < 1.0).collect();
        assert!(sec.len() > 1000 && sec.iter().all(|l| (0.02..=0.3).contains(l)), "{} security lights", sec.len());
        // a block of 16 shops of a commercial street's mix at 21:00: neither all lit nor all dark, in every one of many blocks.
        let mix = [ShopKind::Restaurant, ShopKind::Retail, ShopKind::Bar, ShopKind::Cafe, ShopKind::Retail, ShopKind::Salon, ShopKind::Market, ShopKind::Retail];
        for b in 0..200u32 {
            let lit = (0..16u32).filter(|i| hours(mix[*i as usize % mix.len()], shop_key(&format!("block{b}"), *i)).level_at(21.0) >= 1.0).count();
            assert!(lit > 0 && lit < 16, "block {b}: {lit} of 16 lit");
        }
        // hours on the quarter hour, a positive span, the same for the same key.
        for i in 0..500u32 {
            for k in ShopKind::ALL {
                let h = hours(k, mix32(i));
                assert_eq!(h, hours(k, mix32(i)));
                assert!((0.0..24.0).contains(&h.open) && (0.0..=24.0).contains(&h.span));
                assert!(((h.open / HOURS_STEP_H).fract()).abs() < 1e-9);
            }
        }
        // the switch is a step at the closing time, wrapping past midnight.
        let h = Hours { open: 15.0, span: 11.0, security: 0.05 };
        assert_eq!((h.level_at(14.9), h.level_at(15.0), h.level_at(25.9), h.level_at(1.9), h.level_at(2.1)), (0.05, 1.0, 1.0, 1.0, 0.05));
    }

    /// The colour by trade: warm restaurants and bars, neutral shops, cool
    /// markets and laundromats, every one at unit luminance, a 3000 K shop as
    /// the LED street lamps read.
    #[test]
    fn the_colour_follows_the_trade_from_warm_bars_to_cool_laundromats() {
        let mean_cct = |k: ShopKind| (0..500u32).map(|i| cct(k, mix32(i))).sum::<f64>() / 500.0;
        let order = [ShopKind::Bar, ShopKind::Restaurant, ShopKind::Cafe, ShopKind::Retail, ShopKind::Salon, ShopKind::Pharmacy, ShopKind::Market, ShopKind::Laundromat];
        for w in order.windows(2) {
            assert!(mean_cct(w[0]) < mean_cct(w[1]), "{:?} {} vs {:?} {}", w[0], mean_cct(w[0]), w[1], mean_cct(w[1]));
        }
        assert!((2600.0..=2800.0).contains(&mean_cct(ShopKind::Restaurant)) && mean_cct(ShopKind::Laundromat) >= 5000.0);
        // blue over red rises with the colour temperature; luminance 1.
        let br = |c: [f64; 3]| c[2] / c[0];
        let (bar, rest, shop, laundry) = (colour(ShopKind::Bar, 1), adapted_source(2700.0, SF_LED_CCT_K, 0.66), adapted_source(3500.0, SF_LED_CCT_K, 0.66), adapted_source(5500.0, SF_LED_CCT_K, 0.66));
        println!("bar {bar:.3?}, 2700 K {rest:.3?}, 3500 K {shop:.3?}, 5500 K {laundry:.3?}");
        assert!(br(rest) < br(shop) && br(shop) < br(laundry));
        assert!(laundry[2] > laundry[0], "a 5500 K laundromat reads bluish white beside the 3000 K street");
        assert!(rest[0] > rest[2], "a 2700 K restaurant reads warm");
        for k in ShopKind::ALL {
            for i in 0..50u32 {
                let c = colour(k, mix32(i));
                assert!((lum(c) - 1.0).abs() < 1e-9 && c.iter().all(|v| *v >= 0.0), "{k:?}: {c:?}");
            }
        }
        // a 3000 K shop is the street lamps' own colour (eo-sky lights::led_street_lamp_color).
        let lamp = crate::lights::led_street_lamp_color();
        let same = adapted_source(SF_LED_CCT_K, SF_LED_CCT_K, degree_of_adaptation(NIGHT_ADAPTING_CD_M2, DARK_SURROUND_F));
        assert!((0..3).all(|i| (lamp[i] - same[i]).abs() < 1e-6), "{lamp:?} vs {same:?}");
    }

    /// The luminance by trade: a bar's window dim, a market's bright, inside
    /// the sheet's bounds (over a lit flat window's ~6 cd/m2, under a
    /// storefront sign's 100-450 cd/m2).
    #[test]
    fn the_windows_luminance_follows_the_trade() {
        for k in ShopKind::ALL {
            for i in 0..200u32 {
                let l = luminance(k, mix32(i));
                assert!((5.0..=200.0).contains(&l), "{k:?}: {l}");
            }
        }
        let mean = |k: ShopKind| (0..500u32).map(|i| luminance(k, mix32(i))).sum::<f64>() / 500.0;
        assert!(mean(ShopKind::Bar) < mean(ShopKind::Restaurant) && mean(ShopKind::Restaurant) < mean(ShopKind::Retail) && mean(ShopKind::Retail) < mean(ShopKind::Market));
        // a 2.4 m x 1.5 m retail bay: ~270 cd, reaching 1 lux at ~16 m.
        let cd = 75.0 * 3.6;
        assert!((15.0..18.0).contains(&reach_m(cd, 1.0)));
    }

    /// (stage N2) One vacancy rule: a shop is vacant exactly when its hours
    /// never light it (dark, or its work light at `VACANT_LIGHT`), per shop
    /// (two tenancies of one building differ), at `VACANT_SHARE`; the work
    /// light only on a vacant front.
    #[test]
    fn the_vacancy_rule_is_the_hours_rule() {
        let (mut vacant_n, mut work_n, n) = (0usize, 0usize, 20_000u32);
        for i in 0..n {
            for k in ShopKind::ALL {
                let key = mix32(i.wrapping_mul(2654435761));
                let h = hours(k, key);
                let v = vacant(k, key);
                assert_eq!(v, h.span == 0.0, "{k:?} {key}: vacant {v}, hours {h:?}");
                assert_eq!(work_light(k, key), v && h.security > 0.0);
                if v {
                    assert!(h.security == 0.0 || h.security == VACANT_LIGHT);
                    assert!((0..24).all(|hr| h.level_at(hr as f64) <= VACANT_LIGHT), "a vacant shop is never lit");
                }
                if k == ShopKind::Retail {
                    vacant_n += v as usize;
                    work_n += work_light(k, key) as usize;
                }
            }
        }
        let share = vacant_n as f64 / n as f64;
        println!("vacant {share:.4} (rule {VACANT_SHARE}), with a work light {:.3} of them", work_n as f64 / vacant_n.max(1) as f64);
        assert!((share - VACANT_SHARE).abs() < 0.006, "{share}");
        assert!((work_n as f64 / vacant_n as f64 - VACANT_WORK_SHARE).abs() < 0.05);
        // per shop: the tenancies of one building are not all vacant together.
        let mixed = (0..2000u32).filter(|b| {
            let v: Vec<bool> = (0..6u32).map(|t| vacant(ShopKind::Retail, shop_key(&format!("ds{b}"), t))).collect();
            v.iter().any(|x| *x) && v.iter().any(|x| !*x)
        });
        assert!(mixed.count() > 400, "buildings with a vacant and an occupied shop");
    }

    /// (stage N2) The trade's fittings: food shops dine, bars have a back bar,
    /// markets, liquor stores and hardware are shelved, a clothes shop racked,
    /// a gallery sparse; every kind has one.
    #[test]
    fn a_shops_fittings_follow_its_trade() {
        assert_eq!(fit_of(ShopKind::Restaurant, "cafe", "Golden Dragon|NOODLES"), Fit::Dining);
        assert_eq!(fit_of(ShopKind::Cafe, "bakery", ""), Fit::Dining);
        assert_eq!(fit_of(ShopKind::Bar, "bar", "The Tap Room"), Fit::Bar);
        assert_eq!(fit_of(ShopKind::Market, "grocery", ""), Fit::Shelves);
        assert_eq!(fit_of(ShopKind::Liquor, "grocery", "Corner|LIQUOR"), Fit::Shelves);
        assert_eq!(goods_of(ShopKind::Liquor, "grocery", "Corner|LIQUOR"), 1);
        assert_eq!(fit_of(ShopKind::Retail, "", "Mission|HARDWARE"), Fit::Shelves);
        assert_eq!(goods_of(ShopKind::Retail, "", "Mission|HARDWARE"), 3);
        assert_eq!(fit_of(ShopKind::Retail, "books", ""), Fit::Shelves);
        assert_eq!(goods_of(ShopKind::Retail, "books", ""), 4);
        assert_eq!(fit_of(ShopKind::Retail, "", "Valencia|FLORIST"), Fit::Shelves);
        assert_eq!(goods_of(ShopKind::Retail, "", "Valencia|FLORIST"), 5);
        assert_eq!(fit_of(ShopKind::Retail, "", ""), Fit::Racks);
        assert_eq!(fit_of(ShopKind::Retail, "", "Castro|SHOES"), Fit::Racks);
        assert_eq!(fit_of(ShopKind::Retail, "studio", ""), Fit::Gallery);
        assert_eq!(fit_of(ShopKind::Retail, "", "Noe|FURNITURE"), Fit::Gallery);
        assert_eq!(fit_of(ShopKind::Salon, "cleaners", ""), Fit::Cleaners);
        assert_eq!(fit_of(ShopKind::Salon, "", "Bay|DENTAL"), Fit::Clinic);
        assert_eq!(fit_of(ShopKind::Salon, "", "Sam|BARBER"), Fit::Salon);
        assert_eq!(fit_of(ShopKind::Laundromat, "cleaners", "LAUNDRY"), Fit::Laundry);
        assert_eq!(fit_of(ShopKind::Fitness, "studio", "GYM"), Fit::Fitness);
        let fits: std::collections::BTreeSet<Fit> = ShopKind::ALL.iter().map(|k| fit_of(*k, "", "")).collect();
        assert!(fits.len() >= 7, "{fits:?}");
        assert_eq!(Fit::ALL.len(), 10);
    }

    #[test]
    fn a_shops_kind_comes_from_its_fascia_then_its_type() {
        assert_eq!(kind_of("cafe", "RESTAURANT"), ShopKind::Restaurant);
        assert_eq!(kind_of("cafe", "Nob Hill Cafe"), ShopKind::Cafe);
        assert_eq!(kind_of("cafe", "Golden Dragon|NOODLES"), ShopKind::Restaurant);
        assert_eq!(kind_of("bar", "The Tap Room"), ShopKind::Bar);
        assert_eq!(kind_of("grocery", "Corner|LIQUOR"), ShopKind::Liquor);
        assert_eq!(kind_of("cleaners", "LAUNDRY"), ShopKind::Laundromat);
        assert_eq!(kind_of("cleaners", ""), ShopKind::Salon);
        assert_eq!(kind_of("grocery", ""), ShopKind::Market);
        assert_eq!(kind_of("books", ""), ShopKind::Retail);
        assert_eq!(kind_of("studio", "GYM"), ShopKind::Fitness);
        assert_eq!(kind_of("", ""), ShopKind::Retail);
        assert_ne!(shop_key("ds10009", 0), shop_key("ds10009", 1));
        assert_eq!(shop_key("ds10009", 0), shop_key("ds10009", 0));
    }
}
