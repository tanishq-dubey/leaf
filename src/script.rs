//! Subscripts and superscripts in a line of PDF text, as Unicode: an `a` set
//! small and low after a `P` reads "Pₐ", where plain extraction says "Pa".
//! A line that text can't hold (a subscript letter Unicode lacks, a fraction,
//! a root) comes back as `Reading::Image`, for the caller to draw instead.

use std::collections::HashMap;

/// A glyph is a script when it's at most this share of the body size...
const SCRIPT_SIZE: f32 = 0.85;
/// ...and a script of a script when it's at most this share: no Unicode for that.
const NESTED_SIZE: f32 = 0.6;
/// A script sits at least this share of the body size off the baseline.
const MIN_SHIFT: f32 = 0.08;
/// Scripts one above the other (a fraction, limits under a sum) overlap this much sideways.
const STACKED_SHARE: f32 = 0.5;
/// Glyph sizes within this many points are one size.
const SIZE_STEP: f32 = 0.5;

/// Accents TeX sets before the letter they sit on, and the combining mark for each.
const ACCENTS: &[(char, char)] = &[
    ('\u{02C6}', '\u{0302}'), // ˆ
    ('\u{02DC}', '\u{0303}'), // ˜
    ('\u{02C9}', '\u{0304}'), // ˉ
    ('\u{00AF}', '\u{0304}'), // ¯
    ('\u{02C7}', '\u{030C}'), // ˇ
    ('\u{02D9}', '\u{0307}'), // ˙
    ('\u{00A8}', '\u{0308}'), // ¨
];
/// Operators that take limits above and below, which text can't put there.
const BIG_OPERATORS: &str = "∑∏∐∫∬∭∮⋀⋁⋂⋃⨀⨁⨂⨆";
/// Marks that can be spelled in a script, where Unicode has no small form of a letter beside them.
const SPELLED_IN_SCRIPT: &str = "+−-=(),.′∗";
/// Marks in a script that read the same small, or need no change.
const KEPT_IN_SCRIPT: &str = ",.′∗†‡'*";
const VARIATION_SELECTORS: std::ops::RangeInclusive<char> = '\u{FE00}'..='\u{FE0F}';
const PRIVATE_USE: std::ops::RangeInclusive<char> = '\u{E000}'..='\u{F8FF}';
const REPLACEMENT: char = '\u{FFFD}';
const ROOT: char = '√';

/// Whether a glyph comes from a font of stretchy pieces: big brackets, braces, roots.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Form {
    Plain,
    Stretchy,
}

/// One character of a line as it is set: where, how big, and from what.
#[derive(Debug, Clone, Copy)]
pub struct Glyph {
    pub ch: char,
    pub size: f32,
    /// Height of its baseline; larger is lower on the page.
    pub base: f32,
    pub left: f32,
    pub right: f32,
    pub form: Form,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Reading {
    Text(String),
    /// Text can't show this line as it's set.
    Image,
}

/// How a glyph sits against the line's baseline.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Rise {
    Level,
    Up,
    Down,
}

/// The line as text with its scripts written out, or `Image` if it can't be.
pub fn read(glyphs: &[Glyph]) -> Reading {
    match written(glyphs) {
        Some(text) => Reading::Text(text),
        None => Reading::Image,
    }
}

fn written(glyphs: &[Glyph]) -> Option<String> {
    if glyphs.iter().any(|g| odd(g.ch)) {
        return None;
    }

    let rises = rises(glyphs)?;
    let scripted = rises.iter().any(|&r| r != Rise::Level);
    if scripted && glyphs.iter().any(|g| BIG_OPERATORS.contains(g.ch)) {
        return None;
    }
    if stacked(glyphs, &rises) {
        return None;
    }

    write_runs(glyphs, &rises)
}

/// The glyphs as text, each run of scripts on its own; `None` if a script
/// has a mark that neither Unicode nor `_b` can say.
fn write_runs(glyphs: &[Glyph], rises: &[Rise]) -> Option<String> {
    let mut out = String::new();
    let mut at = 0;
    while at < glyphs.len() {
        let rise = rises[at];
        let len = rises[at..].iter().take_while(|&&r| r == rise).count();
        let run: Vec<char> = glyphs[at..at + len].iter().map(|g| g.ch).collect();
        out.push_str(&run_text(&run, rise)?);
        at += len;
    }
    Some(accented(&out))
}

/// A run of glyphs at one rise: in Unicode scripts if there is one for every
/// glyph ("ᵢ₊₁"), else spelled out ("_b", "_{b′}", "^{k}").
fn run_text(run: &[char], rise: Rise) -> Option<String> {
    let unicode = |ch: char| match rise {
        Rise::Level => Some(ch),
        Rise::Up => raised(ch),
        Rise::Down => lowered(ch),
    };
    if let Some(text) = run.iter().map(|&ch| unicode(ch)).collect() {
        return Some(text);
    }

    if !run
        .iter()
        .all(|&ch| ch.is_alphanumeric() || SPELLED_IN_SCRIPT.contains(ch))
    {
        return None;
    }
    let mark = if rise == Rise::Up { '^' } else { '_' };
    let body: String = run.iter().collect();
    match run.len() {
        1 => Some(format!("{mark}{body}")),
        _ => Some(format!("{mark}{{{body}}}")),
    }
}

/// A character text has no way to show.
fn odd(ch: char) -> bool {
    ch == REPLACEMENT || ch == ROOT || PRIVATE_USE.contains(&ch)
}

/// How each glyph sits; `None` if one is a script of a script, or stretchy.
fn rises(glyphs: &[Glyph]) -> Option<Vec<Rise>> {
    let Some((body, base)) = body_and_base(glyphs) else {
        return Some(vec![Rise::Level; glyphs.len()]);
    };

    glyphs
        .iter()
        .map(|g| {
            if g.ch.is_whitespace() {
                return Some(Rise::Level);
            }
            // A wide accent is stretchy, and so is the invisible mark after it.
            let accent = ACCENTS.iter().any(|&(mark, _)| mark == g.ch)
                || VARIATION_SELECTORS.contains(&g.ch);
            if g.form == Form::Stretchy && !accent {
                return None;
            }

            let shift = g.base - base;
            let small = g.size <= body * SCRIPT_SIZE;
            if !small || shift.abs() < body * MIN_SHIFT {
                return Some(Rise::Level);
            }
            if g.size <= body * NESTED_SIZE {
                return None;
            }
            Some(if shift < 0.0 { Rise::Up } else { Rise::Down })
        })
        .collect()
}

/// The size most of the line is set in, and the height of its baseline.
fn body_and_base(glyphs: &[Glyph]) -> Option<(f32, f32)> {
    let letters = || glyphs.iter().filter(|g| !g.ch.is_whitespace());

    let mut counts: HashMap<i32, usize> = HashMap::new();
    for g in letters() {
        *counts
            .entry((g.size / SIZE_STEP).round() as i32)
            .or_default() += 1;
    }
    let (&step, _) = counts.iter().max_by_key(|&(step, n)| (*n, *step))?;
    let body = step as f32 * SIZE_STEP;

    let mut bases: Vec<f32> = letters()
        .filter(|g| (g.size - body).abs() <= SIZE_STEP)
        .map(|g| g.base)
        .collect();
    bases.sort_by(f32::total_cmp);
    Some((body, *bases.get(bases.len() / 2)?))
}

/// Whether a raised glyph sits over a lowered one, as in a fraction or limits.
fn stacked(glyphs: &[Glyph], rises: &[Rise]) -> bool {
    let of = |wanted: Rise| {
        glyphs
            .iter()
            .zip(rises)
            .filter(move |&(_, &rise)| rise == wanted)
            .map(|(g, _)| g)
    };

    of(Rise::Up).any(|up| {
        of(Rise::Down).any(|down| {
            let across = up.right.min(down.right) - up.left.max(down.left);
            let narrower = (up.right - up.left).min(down.right - down.left);
            across > narrower * STACKED_SHARE
        })
    })
}

/// `ch` as a superscript.
fn raised(ch: char) -> Option<char> {
    Some(match ch {
        '0' => '⁰',
        '1' => '¹',
        '2' => '²',
        '3' => '³',
        '4' => '⁴',
        '5' => '⁵',
        '6' => '⁶',
        '7' => '⁷',
        '8' => '⁸',
        '9' => '⁹',
        '+' => '⁺',
        '-' | '−' => '⁻',
        '=' => '⁼',
        '(' => '⁽',
        ')' => '⁾',
        'a' => 'ᵃ',
        'b' => 'ᵇ',
        'c' => 'ᶜ',
        'd' => 'ᵈ',
        'e' => 'ᵉ',
        'f' => 'ᶠ',
        'g' => 'ᵍ',
        'h' => 'ʰ',
        'i' => 'ⁱ',
        'j' => 'ʲ',
        'k' => 'ᵏ',
        'l' => 'ˡ',
        'm' => 'ᵐ',
        'n' => 'ⁿ',
        'o' => 'ᵒ',
        'p' => 'ᵖ',
        'r' => 'ʳ',
        's' => 'ˢ',
        't' => 'ᵗ',
        'u' => 'ᵘ',
        'v' => 'ᵛ',
        'w' => 'ʷ',
        'x' => 'ˣ',
        'y' => 'ʸ',
        'z' => 'ᶻ',
        'A' => 'ᴬ',
        'B' => 'ᴮ',
        'D' => 'ᴰ',
        'E' => 'ᴱ',
        'G' => 'ᴳ',
        'H' => 'ᴴ',
        'I' => 'ᴵ',
        'J' => 'ᴶ',
        'K' => 'ᴷ',
        'L' => 'ᴸ',
        'M' => 'ᴹ',
        'N' => 'ᴺ',
        'O' => 'ᴼ',
        'P' => 'ᴾ',
        'R' => 'ᴿ',
        'T' => 'ᵀ',
        'U' => 'ᵁ',
        'V' => 'ⱽ',
        'W' => 'ᵂ',
        c if c.is_whitespace() || KEPT_IN_SCRIPT.contains(c) => c,
        _ => return None,
    })
}

/// `ch` as a subscript.
fn lowered(ch: char) -> Option<char> {
    Some(match ch {
        '0' => '₀',
        '1' => '₁',
        '2' => '₂',
        '3' => '₃',
        '4' => '₄',
        '5' => '₅',
        '6' => '₆',
        '7' => '₇',
        '8' => '₈',
        '9' => '₉',
        '+' => '₊',
        '-' | '−' => '₋',
        '=' => '₌',
        '(' => '₍',
        ')' => '₎',
        'a' => 'ₐ',
        'e' => 'ₑ',
        'h' => 'ₕ',
        'i' => 'ᵢ',
        'j' => 'ⱼ',
        'k' => 'ₖ',
        'l' => 'ₗ',
        'm' => 'ₘ',
        'n' => 'ₙ',
        'o' => 'ₒ',
        'p' => 'ₚ',
        'r' => 'ᵣ',
        's' => 'ₛ',
        't' => 'ₜ',
        'u' => 'ᵤ',
        'v' => 'ᵥ',
        'x' => 'ₓ',
        'β' => 'ᵦ',
        'γ' => 'ᵧ',
        'ρ' => 'ᵨ',
        'φ' => 'ᵩ',
        'χ' => 'ᵪ',
        c if c.is_whitespace() || KEPT_IN_SCRIPT.contains(c) => c,
        _ => return None,
    })
}

/// Accents moved onto the letter after them: "ˆP" (the hat comes first in the
/// PDF) is "P̂". An accent with no letter after it stays as it is.
fn accented(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        let Some(&(_, mark)) = ACCENTS.iter().find(|&&(accent, _)| accent == ch) else {
            out.push(ch);
            continue;
        };

        // The wide hat is followed by a variation selector that shows as nothing.
        while chars.next_if(|c| VARIATION_SELECTORS.contains(c)).is_some() {}
        match chars.next_if(|c| c.is_alphabetic()) {
            Some(letter) => {
                out.push(letter);
                out.push(mark);
            }
            None => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const BODY: f32 = 10.0;
    const SMALL: f32 = 7.0;
    const BASE: f32 = 100.0;
    const RAISE: f32 = -3.5;
    const LOWER: f32 = 2.0;

    /// Glyphs left to right: the text, then each script after it.
    fn glyph(ch: char, size: f32, shift: f32, left: f32) -> Glyph {
        Glyph {
            ch,
            size,
            base: BASE + shift,
            left,
            right: left + size * 0.5,
            form: Form::Plain,
        }
    }

    /// Text at body size from `left`, one glyph per char.
    fn level(text: &str, left: f32) -> Vec<Glyph> {
        text.chars()
            .enumerate()
            .map(|(n, ch)| glyph(ch, BODY, 0.0, left + n as f32 * 5.0))
            .collect()
    }

    fn scripts(text: &str, shift: f32, left: f32) -> Vec<Glyph> {
        text.chars()
            .enumerate()
            .map(|(n, ch)| glyph(ch, SMALL, shift, left + n as f32 * 3.5))
            .collect()
    }

    fn text(glyphs: &[Glyph]) -> String {
        match read(glyphs) {
            Reading::Text(text) => text,
            Reading::Image => "<image>".to_owned(),
        }
    }

    #[test]
    fn plain_text_is_unchanged() {
        assert_eq!(text(&level("a plain line", 0.0)), "a plain line");
        assert_eq!(text(&[]), "");
    }

    #[test]
    fn a_low_small_glyph_is_a_subscript() {
        let line = [level("P", 0.0), scripts("a", LOWER, 5.0)].concat();

        assert_eq!(text(&line), "Pₐ");
    }

    #[test]
    fn a_run_of_subscripts_is_written_out() {
        let line = [level("the s", 0.0), scripts("i+1", LOWER, 25.0)].concat();

        assert_eq!(text(&line), "the sᵢ₊₁");
    }

    #[test]
    fn a_high_small_glyph_is_a_superscript() {
        let line = [
            level("x", 0.0),
            scripts("2", RAISE, 5.0),
            level(" + y", 9.0),
        ]
        .concat();

        assert_eq!(text(&line), "x² + y");
    }

    #[test]
    fn a_small_glyph_on_the_baseline_stays_level() {
        let line = [level("note", 0.0), scripts("a", 0.0, 20.0)].concat();

        assert_eq!(text(&line), "notea");
    }

    #[test]
    fn a_script_unicode_lacks_is_spelled_out() {
        let one = [level("the P", 0.0), scripts("b", LOWER, 25.0)].concat();
        let many = [level("the P", 0.0), scripts("b′", LOWER, 25.0)].concat();
        let up = [level("the D", 0.0), scripts("q", RAISE, 25.0)].concat();

        assert_eq!(text(&one), "the P_b");
        assert_eq!(text(&many), "the P_{b′}");
        assert_eq!(text(&up), "the D^q");
    }

    #[test]
    fn a_script_with_a_mark_text_cannot_say_makes_the_line_an_image() {
        let line = [level("the A", 0.0), scripts("x∈S", LOWER, 25.0)].concat();

        assert_eq!(read(&line), Reading::Image);
    }

    #[test]
    fn marks_in_a_script_that_read_the_same_are_kept() {
        let line = [level("set A", 0.0), scripts("i,j", LOWER, 25.0)].concat();

        assert_eq!(text(&line), "set Aᵢ,ⱼ");
    }

    #[test]
    fn a_script_of_a_script_makes_the_line_an_image() {
        let tiny = glyph('2', 5.0, LOWER + 2.0, 12.0);
        let line = [level("x", 0.0), scripts("a", LOWER, 5.0), vec![tiny]].concat();

        assert_eq!(read(&line), Reading::Image);
    }

    #[test]
    fn a_fraction_stacked_in_the_line_makes_it_an_image() {
        let line = [
            level("see ", 0.0),
            scripts("1", RAISE, 20.0),
            scripts("2", LOWER, 20.0),
        ]
        .concat();

        assert_eq!(read(&line), Reading::Image);
    }

    #[test]
    fn a_stretchy_glyph_makes_the_line_an_image() {
        let mut brace = glyph('{', BODY, 0.0, 6.0);
        brace.form = Form::Stretchy;
        let line = [level("x", 0.0), vec![brace]].concat();

        assert_eq!(read(&line), Reading::Image);
    }

    #[test]
    fn limits_on_a_sum_make_the_line_an_image() {
        let line = [
            level("sum ∑", 0.0),
            scripts("i", LOWER, 30.0),
            scripts("n", RAISE, 30.0),
        ]
        .concat();

        assert_eq!(read(&line), Reading::Image);
    }

    #[test]
    fn a_root_or_a_missing_glyph_makes_the_line_an_image() {
        assert_eq!(read(&level("√x", 0.0)), Reading::Image);
        assert_eq!(read(&level("a\u{FFFD}b", 0.0)), Reading::Image);
        assert_eq!(read(&level("a\u{E001}b", 0.0)), Reading::Image);
    }

    #[test]
    fn an_accent_moves_onto_the_letter_after_it() {
        let mut hat = glyph('\u{02C6}', BODY, 0.0, 0.0);
        let mut selector = glyph('\u{FE00}', BODY, 0.0, 2.0);
        hat.form = Form::Stretchy;
        selector.form = Form::Stretchy;
        let line = [
            level("the ", 0.0),
            vec![hat],
            vec![selector],
            level("P", 25.0),
            scripts("a", LOWER, 30.0),
        ]
        .concat();

        assert_eq!(text(&line), "the P\u{0302}ₐ");
    }

    #[test]
    fn an_accent_with_nothing_to_sit_on_stays() {
        assert_eq!(accented("a ˆ b"), "a ˆ b");
        assert_eq!(accented("ˆ"), "ˆ");
    }
}
