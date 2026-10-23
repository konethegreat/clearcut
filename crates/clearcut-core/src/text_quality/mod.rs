// Written by Kone Tshivhinda & Claude
//! Can the text layer of a page be trusted? (ADR-0016 §4, the detection half of P2-20.)
//!
//! A PDF can carry a text layer that exists and is gibberish: a font whose glyph-to-character
//! mapping is scrambled, so the page *looks* right and *copies* wrong, or digits that are
//! swapped for other digits. "Has a text layer" is therefore not good enough. The worker
//! reports **facts** (the fonts a page was drawn with, and, when asked, how much of the
//! text layer OCR also read); this module measures the text itself and decides.
//!
//! The decision lives here and not in the worker because it is a business rule: the
//! thresholds will be tuned on the gold set, and changing them should not need a Python
//! release. Everything is integer arithmetic, so the same evidence always gives the same
//! answer; shares are in thousandths ("per mille"), like the worker's own.
//!
//! The answer is a verdict and the reasons for it, in a fixed vocabulary. It never holds any
//! of the text (the text may be an invoice with personal data in it, and a reason is logged).
//!
//! **All thresholds are provisional** until there are real samples of obfuscated PDFs
//! (P2-20) and a gold-set run; each says so where it is defined.

#[cfg(test)]
mod tests;

use serde::{Deserialize, Serialize};

// ----------------------------------------------------------------- provisional thresholds ----

/// Text with this share (in percent) or more of replacement or private-use characters is
/// `Unusable`: those are what a failed mapping leaves behind. Provisional.
pub const UNUSABLE_BAD_CHARACTER_PERCENT: usize = 5;
/// An OCR agreement (per mille) below this means the text layer does not say what a reader
/// sees: `Unusable`. Provisional.
pub const OCR_UNUSABLE_BELOW: u16 = 500;
/// An OCR agreement (per mille) from this up settles it: `Good`, whatever the fonts say.
/// Between the two it is `NeedsCheck`. Provisional.
pub const OCR_GOOD_FROM: u16 = 850;
/// More than this share (per mille) of words of three letters or more without a vowel is
/// "many". Invoices do have some (BTW, Ltd), so the limit is generous. Provisional.
pub const MANY_VOWELLESS_ABOVE_PER_MILLE: usize = 300;

/// The words an invoice almost always has, in English and Afrikaans (ADR-0016 §4).
/// Matched as whole words, ignoring case. Provisional list.
pub const KEYWORDS: &[&str] = &[
    "invoice",
    "tax",
    "vat",
    "total",
    "date",
    "amount",
    "quantity",
    "price",
    "description",
    "due",
    "faktuur",
    "btw",
    "totaal",
    "datum",
    "bedrag",
    "hoeveelheid",
    "prys",
    "beskrywing",
    "vervaldatum",
];

// ------------------------------------------------------------------------- font facts ----

/// What kind of font a page was drawn with, as the worker reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FontKind {
    Type1,
    #[serde(rename = "truetype")]
    TrueType,
    Type0,
    Type3,
    Other,
}

/// How a font maps character codes to characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FontEncoding {
    None,
    #[serde(rename = "winansi")]
    WinAnsi,
    #[serde(rename = "macroman")]
    MacRoman,
    Standard,
    Identity,
    Custom,
    Other,
}

/// One font a page can draw with, exactly as the worker states it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FontFact {
    pub kind: FontKind,
    pub has_to_unicode: bool,
    pub encoding: FontEncoding,
    pub embedded: bool,
}

impl FontFact {
    /// Whether the characters this font draws may not be recoverable as text: a Type 3
    /// font draws its own shapes, and a font with a custom or identity encoding and no
    /// map back to Unicode gives codes that mean nothing outside the font.
    pub fn is_risky(&self) -> bool {
        self.kind == FontKind::Type3
            || (matches!(self.encoding, FontEncoding::Custom | FontEncoding::Identity)
                && !self.has_to_unicode)
    }
}

// --------------------------------------------------------------------------- measures ----

/// What the text itself looks like. Counts of characters are of non-white-space characters.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct TextMeasures {
    /// Characters that are not white space.
    pub characters: usize,
    /// Replacement characters (U+FFFD) and private-use characters: what a failed mapping leaves.
    pub replacement_or_private_use: usize,
    /// Control characters other than white space.
    pub control: usize,
    pub letters: usize,
    pub digits: usize,
    /// ASCII punctuation and symbols, currency signs and the usual typographic punctuation.
    pub punctuation: usize,
    /// Words of three letters or more written in Latin letters (other scripts are not
    /// judged by Latin vowels).
    pub words: usize,
    /// Of those, words with no vowel (`y` counts as one).
    pub vowelless_words: usize,
    /// Of those, words with a capital after a small letter inside them ("iNvOicE").
    pub mid_word_capital_words: usize,
    /// The longest run of characters without white space.
    pub longest_token: usize,
    /// How many of the [`KEYWORDS`] appear (each counted once).
    pub keyword_hits: usize,
}

fn share(count: usize, whole: usize) -> usize {
    (count * 1000).checked_div(whole).unwrap_or(0)
}

impl TextMeasures {
    pub fn letters_per_mille(&self) -> usize {
        share(self.letters, self.characters)
    }

    pub fn digits_per_mille(&self) -> usize {
        share(self.digits, self.characters)
    }

    pub fn punctuation_per_mille(&self) -> usize {
        share(self.punctuation, self.characters)
    }

    pub fn vowelless_per_mille(&self) -> usize {
        share(self.vowelless_words, self.words)
    }

    pub fn mid_word_capital_per_mille(&self) -> usize {
        share(self.mid_word_capital_words, self.words)
    }
}

fn is_private_use(c: char) -> bool {
    matches!(c, '\u{E000}'..='\u{F8FF}' | '\u{F0000}'..='\u{FFFFD}' | '\u{100000}'..='\u{10FFFD}')
}

fn is_punctuation_or_symbol(c: char) -> bool {
    c.is_ascii_punctuation()
        || matches!(
            c,
            '\u{A1}'..='\u{BF}'
                | '\u{D7}'
                | '\u{F7}'
                | '\u{2010}'..='\u{2027}'
                | '\u{2030}'..='\u{205E}'
                | '\u{20A0}'..='\u{20CF}'
                | '\u{3001}'..='\u{303F}'
        )
}

fn is_latin_letter(c: char) -> bool {
    // (Only ever asked about letters, so the in-between signs × and ÷ do not come up.)
    c.is_ascii_alphabetic() || ('\u{C0}'..='\u{24F}').contains(&c)
}

fn is_vowel(c: char) -> bool {
    c.to_lowercase().any(|l| {
        matches!(
            l,
            'a' | 'e' | 'i' | 'o' | 'u' | 'y'
                | 'à'..='å'
                | 'æ'
                | 'è'..='ï'
                | 'ò'..='ö'
                | 'ø'
                | 'ù'..='ü'
                | 'ý'
                | 'ÿ'
                | 'œ'
        )
    })
}

/// A run of letters being read: what is needed to judge it once it ends.
#[derive(Default)]
struct Run {
    letters: usize,
    all_latin: bool,
    has_vowel: bool,
    mid_capital: bool,
    previous_lower: bool,
}

impl Run {
    fn new() -> Self {
        Run {
            all_latin: true,
            ..Run::default()
        }
    }

    fn push(&mut self, c: char) {
        self.letters += 1;
        self.all_latin &= is_latin_letter(c);
        self.has_vowel |= is_vowel(c);
        if self.previous_lower && c.is_uppercase() {
            self.mid_capital = true;
        }
        self.previous_lower = c.is_lowercase();
    }

    fn finish(self, m: &mut TextMeasures) {
        if self.letters >= 3 && self.all_latin {
            m.words += 1;
            m.vowelless_words += usize::from(!self.has_vowel);
            m.mid_word_capital_words += usize::from(self.mid_capital);
        }
    }
}

/// The longest keyword is eleven letters; a longer word cannot be one.
const LONGEST_KEYWORD: usize = 11;

fn keyword_hits(text: &str) -> usize {
    let mut seen = [false; KEYWORDS.len()];
    for token in text.split(|c: char| !c.is_alphanumeric()) {
        if token.is_empty() || token.len() > LONGEST_KEYWORD || !token.is_ascii() {
            continue;
        }
        if let Some(i) = KEYWORDS.iter().position(|k| k.eq_ignore_ascii_case(token)) {
            seen[i] = true;
        }
    }
    seen.iter().filter(|&&hit| hit).count()
}

/// Measures the text. Linear in its length, and it never fails: any text is measurable.
pub fn measure(text: &str) -> TextMeasures {
    let mut m = TextMeasures::default();
    let mut run: Option<Run> = None;
    let mut token_length = 0usize;
    for c in text.chars() {
        if c.is_whitespace() {
            token_length = 0;
        } else {
            m.characters += 1;
            token_length += 1;
            m.longest_token = m.longest_token.max(token_length);
            if c == '\u{FFFD}' || is_private_use(c) {
                m.replacement_or_private_use += 1;
            } else if c.is_control() {
                m.control += 1;
            }
            if c.is_alphabetic() {
                m.letters += 1;
            } else if c.is_numeric() {
                m.digits += 1;
            } else if is_punctuation_or_symbol(c) {
                m.punctuation += 1;
            }
        }
        if c.is_alphabetic() {
            run.get_or_insert_with(Run::new).push(c);
        } else if let Some(finished) = run.take() {
            finished.finish(&mut m);
        }
    }
    if let Some(finished) = run {
        finished.finish(&mut m);
    }
    m.keyword_hits = keyword_hits(text);
    m
}

// ---------------------------------------------------------------------------- verdict ----

/// What to do with a page's text layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TextVerdict {
    /// Use the text layer as the page's text.
    Good,
    /// Check it against OCR, which then makes it `Good` or `Unusable`.
    NeedsCheck,
    /// Do not use the text layer; read the page by OCR instead.
    Unusable,
}

impl TextVerdict {
    pub fn as_str(self) -> &'static str {
        match self {
            TextVerdict::Good => "good",
            TextVerdict::NeedsCheck => "needs_check",
            TextVerdict::Unusable => "unusable",
        }
    }
}

/// Why a verdict is what it is: a fixed vocabulary, with none of the text in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    /// The page has no text at all.
    NoText,
    /// Too many replacement or private-use characters.
    ReplacementCharacters,
    /// OCR read too little of what the text layer says.
    OcrDisagrees,
    /// OCR read some of what the text layer says, not enough to trust it.
    OcrPartlyAgrees,
    /// OCR read nearly all of what the text layer says.
    OcrAgrees,
    /// A font was used whose characters may not map back to text.
    RiskyFont,
    /// None of the usual invoice words is in the text.
    NoKeywords,
    /// Many words have no vowel.
    ManyVowellessWords,
}

impl Reason {
    pub fn as_str(self) -> &'static str {
        match self {
            Reason::NoText => "no_text",
            Reason::ReplacementCharacters => "replacement_characters",
            Reason::OcrDisagrees => "ocr_disagrees",
            Reason::OcrPartlyAgrees => "ocr_partly_agrees",
            Reason::OcrAgrees => "ocr_agrees",
            Reason::RiskyFont => "risky_font",
            Reason::NoKeywords => "no_keywords",
            Reason::ManyVowellessWords => "many_vowelless_words",
        }
    }
}

/// Everything the verdict is made from.
#[derive(Debug, Clone, Copy)]
pub struct TextEvidence<'a> {
    /// The page's text layer.
    pub text: &'a str,
    /// The fonts the page can draw with, from the worker.
    pub fonts: &'a [FontFact],
    /// Per mille of the text layer's words that OCR also read, if OCR was run. A value
    /// above 1000 (the worker never sends one) is taken as 1000.
    pub ocr_agreement: Option<u16>,
}

/// The verdict, why, and the measures it rests on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct QualityAssessment {
    pub verdict: TextVerdict,
    pub reasons: Vec<Reason>,
    pub measures: TextMeasures,
}

/// Decides whether the text layer can be used.
///
/// In order: no text is `Unusable`; so are too many replacement characters, or an OCR
/// agreement under [`OCR_UNUSABLE_BELOW`]. Otherwise an OCR agreement decides (`Good` from
/// [`OCR_GOOD_FROM`], `NeedsCheck` between). With no OCR result, a risky font together with
/// weak text (no invoice words, or many words without a vowel) is `NeedsCheck`; anything
/// else is `Good`.
pub fn assess(evidence: &TextEvidence<'_>) -> QualityAssessment {
    let measures = measure(evidence.text);
    let (verdict, reasons) = decide(&measures, evidence);
    QualityAssessment {
        verdict,
        reasons,
        measures,
    }
}

fn decide(m: &TextMeasures, evidence: &TextEvidence<'_>) -> (TextVerdict, Vec<Reason>) {
    if m.characters == 0 {
        return (TextVerdict::Unusable, vec![Reason::NoText]);
    }
    let agreement = evidence.ocr_agreement.map(|a| a.min(1000));

    let mut unusable = Vec::new();
    if m.replacement_or_private_use * 100 >= m.characters * UNUSABLE_BAD_CHARACTER_PERCENT {
        unusable.push(Reason::ReplacementCharacters);
    }
    if agreement.is_some_and(|a| a < OCR_UNUSABLE_BELOW) {
        unusable.push(Reason::OcrDisagrees);
    }
    if !unusable.is_empty() {
        return (TextVerdict::Unusable, unusable);
    }

    if let Some(a) = agreement {
        return if a >= OCR_GOOD_FROM {
            (TextVerdict::Good, vec![Reason::OcrAgrees])
        } else {
            (TextVerdict::NeedsCheck, vec![Reason::OcrPartlyAgrees])
        };
    }

    let mut doubts = Vec::new();
    if m.keyword_hits == 0 {
        doubts.push(Reason::NoKeywords);
    }
    if m.vowelless_per_mille() > MANY_VOWELLESS_ABOVE_PER_MILLE {
        doubts.push(Reason::ManyVowellessWords);
    }
    if evidence.fonts.iter().any(FontFact::is_risky) && !doubts.is_empty() {
        let mut reasons = vec![Reason::RiskyFont];
        reasons.extend(doubts);
        return (TextVerdict::NeedsCheck, reasons);
    }
    (TextVerdict::Good, Vec::new())
}
