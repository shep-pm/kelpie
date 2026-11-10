//! Minting a token for one repo, and what a worker's token may hold
//!
//! The stand-in GitHub at [`super::fake`] answers the two calls a mint makes, so
//! every test here reads what kelpie asked for and what it was handed back.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use super::api;
use super::fake::{Asked, FakeClock, FakeGithub, FakeSigner, PEM};
use super::tokens::{base64url, signing_input};
use super::*;

// What a token of kelpie's own for koji is asked for with
const KELPIE_KOJI: &str =
    r#"{"permissions":{"issues":"write","pull_requests":"write"},"repositories":["koji"]}"#;

// What a worker's token for koji is asked for with
const WORKER_KOJI: &str = r#"{"permissions":{"contents":"write"},"repositories":["koji"]}"#;

fn slug(s: &str) -> ForgeSlug {
    ForgeSlug::try_from(s.to_owned()).unwrap()
}

// A folder or file others may read, which kelpie must refuse to use.
#[cfg(unix)]
fn open_to_others(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let mode = fs::metadata(path).unwrap().permissions().mode();
    fs::set_permissions(path, fs::Permissions::from_mode(mode | 0o077)).unwrap();
}

#[cfg(unix)]
fn installations_asked(github: &FakeGithub) -> usize {
    (github.asked().iter())
        .filter(|asked| matches!(asked, Asked::Installation(_)))
        .count()
}

#[test]
fn a_token_is_kept_until_five_minutes_before_it_lapses_then_minted_again() {
    let home = tempfile::tempdir().unwrap();
    let clock = FakeClock::at(1_000_000);
    let github = FakeGithub::new(clock.clone(), "shep-pm");
    github.registered(home.path(), "shep-pm/koji");
    let tokens = github.tokens(home.path());
    let koji = slug("shep-pm/koji");

    let first = tokens.token(&koji).unwrap();
    clock.advance(FakeGithub::LIFETIME - 5 * 60 - 1);
    assert_eq!(tokens.token(&koji).unwrap(), first, "still minutes to run");
    assert_eq!(
        github.asked(),
        [
            Asked::Installation("shep-pm/koji".to_owned()),
            Asked::AccessToken(7, KELPIE_KOJI.to_owned()),
        ]
    );

    clock.advance(1);
    let second = tokens.token(&koji).unwrap();

    assert_ne!(second, first);
    assert_eq!(second.expose(), "ghs_test2");
    assert_eq!(
        github.asked().last(),
        Some(&Asked::AccessToken(7, KELPIE_KOJI.to_owned())),
        "the installation is known, so only a token is asked for"
    );
    assert_eq!(github.asked().len(), 3);
}

// A worker's credential on GitHub: it pushes its branch, and the permission to
// comment, review or merge is not in it. Each turn gets one of its own, since a
// turn can run most of an hour, and kelpie's own posts are never made with it.
#[test]
fn a_workers_token_is_for_one_repo_to_push_and_minted_fresh_for_each_call() {
    let home = tempfile::tempdir().unwrap();
    let github = FakeGithub::new(FakeClock::at(1_000_000), "shep-pm");
    github.registered(home.path(), "shep-pm/koji");
    let tokens = github.tokens(home.path());
    let koji = slug("shep-pm/koji");

    let first = tokens.worker_token(&koji).unwrap();
    let second = tokens.worker_token(&koji).unwrap();
    let own = tokens.token(&koji).unwrap();
    let own_again = tokens.token(&koji).unwrap();

    assert_eq!(
        [first.expose(), second.expose(), own.expose()],
        ["ghs_test1", "ghs_test2", "ghs_test3"]
    );
    assert_eq!(own_again, own, "kelpie keeps its own, and never a worker's");
    assert_eq!(
        github.asked(),
        [
            Asked::Installation("shep-pm/koji".to_owned()),
            Asked::AccessToken(7, WORKER_KOJI.to_owned()),
            Asked::Installation("shep-pm/koji".to_owned()),
            Asked::AccessToken(7, WORKER_KOJI.to_owned()),
            Asked::AccessToken(7, KELPIE_KOJI.to_owned()),
        ],
        "a worker's token reads the installation afresh; kelpie's own reuses the one kept"
    );
}

// Changing a workflow file takes the `workflows` permission, and a change to CI
// runs with the repo's secrets, so a worker's token asks for it only where the
// installation holds it.
#[test]
fn a_workers_token_asks_to_change_workflow_files_only_where_the_installation_holds_it() {
    let home = tempfile::tempdir().unwrap();
    let github = FakeGithub::new(FakeClock::at(1_000_000), "shep-pm");
    github.registered(home.path(), "shep-pm/koji");
    let tokens = github.tokens(home.path());
    let koji = slug("shep-pm/koji");

    let without = tokens.worker(&koji).unwrap();
    github.hold_workflows();
    let with = tokens.worker(&koji).unwrap();

    assert!(!without.workflows);
    assert!(with.workflows, "accepted since, and asked for at once");
    let asked_for: Vec<_> = (github.asked().into_iter())
        .filter_map(|asked| match asked {
            Asked::AccessToken(_, body) => Some(body),
            Asked::Installation(_) => None,
        })
        .collect();
    assert_eq!(
        asked_for,
        [
            WORKER_KOJI,
            r#"{"permissions":{"contents":"write","workflows":"write"},"repositories":["koji"]}"#,
        ]
    );
}

// GitHub's answer says what the token holds: pushing and changing workflow files
// are the most a worker is handed.
#[test]
fn a_token_that_changes_workflow_files_and_more_is_never_handed_to_a_worker() {
    let home = tempfile::tempdir().unwrap();
    let github = FakeGithub::new(FakeClock::at(1_000_000), "shep-pm");
    github.registered(home.path(), "shep-pm/koji");
    github.hold_workflows();
    let tokens = github.tokens(home.path());
    let koji = slug("shep-pm/koji");

    assert!(tokens.worker(&koji).is_ok(), "contents and workflows alone");
    for broader in ["issues", "pull_requests"] {
        github.grants_too(broader, "write");
        let err = tokens.worker(&koji).unwrap_err();
        assert!(matches!(err, TokenError::TooBroad(_)), "{broader}: {err:?}");
        assert!(err.to_string().contains(broader), "{err}");
    }
}

// GitHub's answer says what the token holds. A worker is handed one only if that
// is the permission to push, and to change workflow files where the App holds
// it, and nothing else, whatever was asked for.
#[test]
fn a_token_that_holds_more_than_pushing_is_never_handed_to_a_worker() {
    let home = tempfile::tempdir().unwrap();
    let github = FakeGithub::new(FakeClock::at(1_000_000), "shep-pm");
    github.registered(home.path(), "shep-pm/koji");
    let tokens = github.tokens(home.path());
    github.grants_too("pull_requests", "write");

    let err = tokens.worker_token(&slug("shep-pm/koji")).unwrap_err();

    assert_eq!(
        err.to_string(),
        "GitHub made the worker's token hold contents: write, metadata: read, \
         pull_requests: write, which is not the permission to push and nothing else"
    );
}

// An App registered before workers held its tokens has no Contents permission,
// and GitHub answers a request for it with 422.
#[test]
fn an_app_without_the_contents_permission_is_told_apart_from_other_refusals() {
    let home = tempfile::tempdir().unwrap();
    let github = FakeGithub::new(FakeClock::at(1_000_000), "shep-pm");
    github.registered(home.path(), "shep-pm/koji");
    let tokens = github.tokens(home.path());
    tokens.token(&slug("shep-pm/koji")).unwrap();

    github.fail_next(ApiError::Refused(422));
    let err = tokens.worker_token(&slug("shep-pm/koji")).unwrap_err();

    assert!(matches!(err, TokenError::NoContents(_)), "{err:?}");
    assert!(err.to_string().contains("Contents"), "{err}");
}

#[test]
fn an_owner_that_is_no_login_is_refused_before_any_file_is_read() {
    let home = tempfile::tempdir().unwrap();
    let github = FakeGithub::new(FakeClock::at(0), "shep-pm");
    let err = github
        .tokens(home.path())
        .token(&slug("../koji"))
        .unwrap_err();
    assert_eq!(err.to_string(), r#"".." is not a GitHub login"#);
}

#[test]
fn the_jwt_is_the_apps_client_id_issued_a_minute_back_for_nine_minutes() {
    let home = tempfile::tempdir().unwrap();
    let github = FakeGithub::new(FakeClock::at(1_000_000), "shep-pm");
    github.registered(home.path(), "shep-pm/koji");

    github
        .tokens(home.path())
        .token(&slug("shep-pm/koji"))
        .unwrap();

    let claims = r#"{"exp":1000540,"iat":999940,"iss":"Iv23liTEST"}"#;
    let expected = format!(
        "{}.{}.{}",
        base64url(br#"{"alg":"RS256","typ":"JWT"}"#),
        base64url(claims.as_bytes()),
        base64url(FakeSigner::SIGNATURE)
    );
    assert_eq!(github.jwts(), [expected.clone(), expected.clone()]);
    assert_eq!(
        signing_input("Iv23liTEST", Timestamp::from_secs(1_000_000)),
        expected.rsplit_once('.').unwrap().0
    );
}

#[test]
fn no_app_or_no_install_says_which() {
    let home = tempfile::tempdir().unwrap();
    let github = FakeGithub::new(FakeClock::at(0), "shep-pm");
    let tokens = github.tokens(home.path());
    let owner = Owner::try_from("shep-pm").unwrap();
    assert_eq!(
        tokens.token(&slug("shep-pm/koji")),
        Err(TokenError::NoApp(owner))
    );

    github.registered(home.path(), "shep-pm/koji");
    let err = tokens.token(&slug("Shep-PM/golbat")).unwrap_err();

    assert_eq!(
        err.to_string(),
        "kelpie-shep-pm is not installed on Shep-PM/golbat"
    );
    tokens.token(&slug("Shep-PM/Koji")).unwrap();
}

#[test]
fn an_app_not_installed_on_a_repo_is_asked_about_once_in_ten_minutes() {
    let home = tempfile::tempdir().unwrap();
    let clock = FakeClock::at(1_000_000);
    let github = FakeGithub::new(clock.clone(), "shep-pm");
    github.registered(home.path(), "shep-pm/koji");
    let tokens = github.tokens(home.path());
    let golbat = slug("shep-pm/golbat");

    for _ in 0..5 {
        let error = tokens.token(&golbat).unwrap_err();
        assert!(
            matches!(error, TokenError::NotInstalled { .. }),
            "{error:?}"
        );
    }
    assert_eq!(
        installations_asked(&github),
        1,
        "no JWT signed, no call made, after the first"
    );
    assert_eq!(github.jwts().len(), 1);

    clock.advance(10 * 60 - 1);
    tokens.token(&golbat).unwrap_err();
    assert_eq!(
        installations_asked(&github),
        1,
        "still within the ten minutes"
    );

    github.install("shep-pm/golbat", 7);
    clock.advance(1);
    tokens.token(&golbat).unwrap();
    assert_eq!(
        installations_asked(&github),
        2,
        "asked again once the ten minutes passed"
    );
    tokens.token(&golbat).unwrap();
    assert_eq!(installations_asked(&github), 2, "installed now, and kept");
}

#[test]
fn a_replaced_app_never_hands_out_the_old_apps_token() {
    let home = tempfile::tempdir().unwrap();
    let github = FakeGithub::new(FakeClock::at(1_000_000), "shep-pm");
    github.registered(home.path(), "shep-pm/koji");
    let tokens = github.tokens(home.path());
    let koji = slug("shep-pm/koji");
    let old = tokens.token(&koji).unwrap();

    let replaced = App {
        id: 43,
        ..github.app()
    };
    Apps::under(home.path())
        .save(&replaced, &Pem::new(PEM.to_owned()))
        .unwrap();
    let new = tokens.token(&koji).unwrap();

    assert_ne!(new, old);
    assert_eq!(
        github.asked()[2],
        Asked::Installation("shep-pm/koji".to_owned()),
        "the new App's installation is asked for afresh"
    );
}

#[cfg(unix)]
#[test]
fn saving_makes_the_folders_private() {
    let home = tempfile::tempdir().unwrap();
    let github = FakeGithub::new(FakeClock::at(0), "shep-pm");
    let folder = home.path().join("github/shep-pm");
    fs::create_dir_all(&folder).unwrap();
    for open in [home.path().join("github"), folder.clone()] {
        open_to_others(&open);
        assert!(private(&open).is_err(), "{open:?} is open to others");
    }
    let apps = Apps::under(home.path());
    let app = github.app();

    apps.save(&app, &Pem::new(PEM.to_owned())).unwrap();

    let owner = Owner::try_from("shep-pm").unwrap();
    assert!(private(&folder).is_ok(), "{folder:?}");
    assert!(private(&home.path().join("github")).is_ok());
    assert!(private(&apps.key(&owner)).is_ok());
    assert_eq!(apps.get(&owner), Ok(Some(app)));
}

#[cfg(unix)]
#[test]
fn saving_refuses_a_folder_that_is_a_link() {
    let home = tempfile::tempdir().unwrap();
    let github = FakeGithub::new(FakeClock::at(0), "shep-pm");
    let folder = home.path().join("github/shep-pm");
    fs::create_dir(home.path().join("github")).unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(elsewhere.path(), &folder).unwrap();

    assert_eq!(
        Apps::under(home.path()).save(&github.app(), &Pem::new(PEM.to_owned())),
        Err(StoreError::Exposed(folder))
    );
    assert_eq!(fs::read_dir(elsewhere.path()).unwrap().count(), 0);
}

#[cfg(unix)]
#[test]
fn a_key_others_may_read_is_refused() {
    let home = tempfile::tempdir().unwrap();
    let github = FakeGithub::new(FakeClock::at(0), "shep-pm");
    github.registered(home.path(), "shep-pm/koji");
    let key = home.path().join("github/shep-pm/key.pem");
    open_to_others(&key);

    let err = github
        .tokens(home.path())
        .token(&slug("shep-pm/koji"))
        .unwrap_err();

    assert_eq!(err, TokenError::Store(StoreError::Exposed(key)));
    assert_eq!(github.asked(), [], "nothing is signed with an exposed key");
}

// From the example answers in GitHub's REST documentation for each call.
#[test]
fn githubs_answers_are_read() {
    let installation =
        r#"{"id":1,"account":{"login":"octocat"},"app_id":1,"target_type":"Organization"}"#;
    let plain = Installation {
        id: 1,
        workflows: false,
    };
    assert_eq!(api::parse_installation(installation), Ok(plain));
    let accepted = r#"{"id":1,"permissions":{"contents":"write","workflows":"write"}}"#;
    assert_eq!(
        api::parse_installation(accepted),
        Ok(Installation {
            workflows: true,
            ..plain
        })
    );
    let read_only = r#"{"id":1,"permissions":{"workflows":"read"}}"#;
    assert_eq!(api::parse_installation(read_only), Ok(plain));

    let token = r#"{"token":"ghs_16C7e42F292c6912E7710c838347Ae178B4a","expires_at":"2016-07-11T22:14:10Z",
        "permissions":{"issues":"write"},"repository_selection":"selected"}"#;
    let issued = api::parse_token(token).unwrap();
    assert_eq!(
        issued.token.expose(),
        "ghs_16C7e42F292c6912E7710c838347Ae178B4a"
    );
    assert_eq!(issued.expires_at, Timestamp::from_secs(1_468_275_250));
    assert_eq!(
        issued.permissions,
        BTreeMap::from([("issues".to_owned(), "write".to_owned())])
    );
    assert_eq!(
        api::parse_token(r#"{"token":"t"}"#),
        Err(ApiError::Unreadable("expires_at"))
    );
}

#[test]
fn a_redirect_or_a_rate_limit_is_told_from_a_refusal() {
    let none = Cooldown::default();
    assert_eq!(api::refusal(301, "", none), ApiError::Moved);
    assert!(ApiError::Moved.to_string().contains("`owner/name`"));
    assert_eq!(api::refusal(429, "", none), ApiError::RateLimited(none));
    let limited = r#"{"message":"API rate limit exceeded for installation ID 7."}"#;
    let after = Cooldown {
        retry_after: Some(30),
        reset: Some(Timestamp::from_secs(1_790_003_600)),
    };
    assert_eq!(
        api::refusal(403, limited, after),
        ApiError::RateLimited(after)
    );
    let denied = r#"{"message":"Resource not accessible by integration"}"#;
    assert_eq!(api::refusal(403, denied, after), ApiError::Refused(403));
    assert!(ApiError::Refused(502).passes() && ApiError::RateLimited(none).passes());
    assert!(!ApiError::Refused(401).passes() && !ApiError::Moved.passes());
    assert_eq!(
        api::token_request(&slug("shep-pm/koji"), Grant::Kelpie),
        KELPIE_KOJI
    );
}

#[test]
fn origin_of_reads_a_clones_origin_as_a_forge_slug() {
    use crate::worktree::tests::{Dir, git, process_tree};

    let dir = Dir::new("origin-of");
    let tree = process_tree(dir.path());
    let repo = dir.path().join("repo");
    fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "--quiet", "-b", "main"]);

    assert_eq!(origin_of(&tree, &repo), None, "no remote names no repo");

    git(
        &repo,
        &[
            "config",
            "remote.origin.url",
            "https://example.com/shep-pm/koji.git",
        ],
    );
    assert_eq!(origin_of(&tree, &repo), None, "a remote that is not GitHub");

    git(
        &repo,
        &[
            "config",
            "remote.origin.url",
            "git@github.com:shep-pm/koji.git",
        ],
    );
    assert_eq!(origin_of(&tree, &repo), Some(slug("shep-pm/koji")));
}
