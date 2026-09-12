//! The words on the page.
//!
//! Copy lives here rather than inside the templates so that the templates stay
//! structural, and so that changing what wellbe.social *says it stands for* is
//! a reviewable diff in one file rather than an edit buried in markup.

pub struct Principle {
    /// The thing we choose.
    pub over: &'static str,
    /// The thing we are choosing it over.
    pub under: &'static str,
    pub body: &'static str,
}

pub const SLOGAN_LEAD: &str =
    "wellbe.social will help you connect and share with the people in your life.";
pub const SLOGAN_REST: &str = "In a healthy way. On your terms.";

/// The paragraph under the slogan.
pub const HERO_INTRO: &str = "We are going to build a place to keep up with the people who matter to you, \
without farming your attention to pay for it. This page is where we say what we are going to be, before \
we ask you to trust us with anything — and where you put your name down.";

pub const FOOTNOTE: &str = "That first line is how Facebook described itself for years, give or take a tense. \
It was a good promise. We would like to be the ones who actually keep it.";

pub const PRINCIPLES: &[Principle] = &[
    Principle {
        over: "People's wellbeing",
        under: "profit",
        body: "Nobody's attention is for sale here. No advertising, no engagement targets, no feed \
               tuned to keep you scrolling past your bedtime. If the healthy thing for you today is \
               to close the app and go and see someone, the app should help you do exactly that and \
               count it as a success.",
    },
    Principle {
        over: "Valuable discussion",
        under: "quick decisions",
        body: "We are slow on purpose. Decisions about how people relate to each other deserve \
               argument, objection and a night's sleep. We would rather take a season to get a thing \
               right than a weekend to ship it and a decade to undo it.",
    },
    Principle {
        over: "Power split by organisation",
        under: "technical decentralisation",
        body: "We value decentralisation and we will use it where it genuinely serves people. But it \
               is a means, not the point — a distributed protocol can be just as unaccountable as a \
               single server, and much harder to fix. Where the technical answer makes things worse, \
               we split power the older way instead: separate roles, limited mandates, fixed terms, \
               published budgets, and an assembly that can say no.",
    },
    Principle {
        over: "Automation in the rules we are bound by",
        under: "promises about our good intentions",
        body: "Our commitments belong in the statutes, not in a blog post about our values. Where a \
               rule can be executed — who may vote, when a mandate expires, what has to be published \
               before money moves — we want it running by itself, so that it still holds on the day \
               the people in charge would rather it didn't.",
    },
];

pub const GOVERNANCE: &str = "We are a democratically organised non-profit, and we are here for the long term. \
Democracies are slower. They also tend to create more value, for more people, over more years. \
We are making that trade deliberately, with our eyes open, and we would rather tell you about it now \
than have you discover it later.";

/// Shown above the form.
pub const SIGNUP_PROMISE: &str = "We will write to you once, to check this address is really yours, \
and then again when we are ready for you. Nothing else, unless you ask us for it.";
