//! Which heads a pass vouches for, and what it says of one it does not

use super::unread::{Heads, Unread, unread_of};

const FIRST: &str = "aaaaaaa1111111111111111111111111111111aa";
const FIX: &str = "bbbbbbb2222222222222222222222222222222bb";
const LATER: &str = "ccccccc3333333333333333333333333333333cc";

#[test]
fn a_head_no_read_and_no_fix_turn_pushed_is_unread() {
    let mut heads = Heads::default();
    heads.read(FIRST);

    assert_eq!(unread_of(&heads, FIRST, None), None);
    let unread = unread_of(&heads, LATER, None).unwrap();
    assert_eq!(
        unread,
        Unread::Pushed {
            head: LATER.to_owned(),
            last_read: Some(FIRST.to_owned()),
        }
    );
    assert_eq!(
        unread.text(),
        "ccccccc was pushed after the last review read, which was of aaaaaaa, and no reviewer \
         read it."
    );
}

#[test]
fn a_fix_turns_pushed_head_is_vouched_for() {
    let mut heads = Heads::default();
    heads.read(FIRST);
    heads.vouch(FIX, false);

    assert!(heads.vouches_for(FIX));
    assert!(!heads.reviewed(FIX));
    assert_eq!(unread_of(&heads, FIX, None), None);
    assert!(unread_of(&heads, LATER, None).is_some());
}

#[test]
fn a_nit_only_fixs_head_counts_as_read() {
    let mut heads = Heads::default();
    heads.vouch(FIX, true);

    assert!(heads.nit_fixed(FIX));
    assert!(heads.reviewed(FIX));
    assert!(!heads.reviewed(FIRST));
}

#[test]
fn a_pass_that_ended_short_names_why() {
    let mut heads = Heads::default();
    heads.read(FIRST);

    let unread = unread_of(
        &heads,
        FIRST,
        Some(Some("1 of 2 reviewers read it".to_owned())),
    );
    assert_eq!(
        unread.unwrap().text(),
        "The review ended short at aaaaaaa: 1 of 2 reviewers read it."
    );

    let none = unread_of(&Heads::default(), FIRST, Some(None)).unwrap();
    assert_eq!(none.text(), "No reviewer read aaaaaaa.");
}
