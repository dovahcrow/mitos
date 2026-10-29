use super::*;
use view::{current_ref, document::Mode};

#[tokio::test(flavor = "multi_thread")]
async fn two_character_pattern_jumps_to_its_only_match() -> anyhow::Result<()> {
    test_with_config(
        AppBuilder::new().with_file("foo.txt", None),
        (
            indoc! {"\
                one two
                three #[|]#four
                five six
            "},
            "gwsi",
            indoc! {"\
                one two
                three four
                five #[s|]#ix
            "},
        ),
    )
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn labels_pick_between_pattern_matches() -> anyhow::Result<()> {
    test_with_config(
        AppBuilder::new().with_file("foo.txt", None),
        (
            indoc! {"\
                #[|]#one
                two target
                three potato
            "},
            "gwtaa",
            indoc! {"\
                one
                two #[t|]#arget
                three potato
            "},
        ),
    )
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn pattern_matching_ignores_case() -> anyhow::Result<()> {
    test_with_config(
        AppBuilder::new().with_file("foo.txt", None),
        (
            indoc! {"\
                #[|]#one two
                three FOUR
            "},
            "gwfo",
            indoc! {"\
                one two
                three #[F|]#OUR
            "},
        ),
    )
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn pattern_without_matches_keeps_the_cursor() -> anyhow::Result<()> {
    test_with_config(
        AppBuilder::new().with_file("foo.txt", None),
        (
            indoc! {"\
                one #[|]#two
                three four
            "},
            "gwzz",
            indoc! {"\
                one #[t|]#wo
                three four
            "},
        ),
    )
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn select_mode_extends_over_the_whole_match() -> anyhow::Result<()> {
    test_with_config(
        AppBuilder::new().with_file("foo.txt", None),
        (
            indoc! {"\
                #[|]#one two
                three target
            "},
            "vgwta",
            indoc! {"\
                #[one two
                three ta|]#rget
            "},
        ),
    )
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn pattern_jump_only_searches_the_focused_split() -> anyhow::Result<()> {
    let other = temp_file_with_contents("target\n")?;
    let mut app = AppBuilder::new()
        .with_input_text(indoc! {"\
            alpha #[g|]#amma
        "})
        .build()?;

    test_key_sequences(
        &mut app,
        vec![
            (
                Some(&format!(
                    "<C-w>v:o {}<ret><C-w>hgwta",
                    other.path().display()
                )),
                Some(&|app| {
                    let (view, doc) = current_ref!(app.editor);
                    assert_eq!(2, app.editor.tree.views().count());
                    // The only match lives in the other split, so the focused
                    // view keeps its document and selection.
                    assert_eq!("alpha gamma\n", doc.text().slice(..).to_string());
                    assert_eq!(6, doc.selection(view.id).primary().anchor);
                    assert_eq!(7, doc.selection(view.id).primary().head);
                }),
            ),
            (
                Some(":qa!<ret>"),
                Some(&|app| assert_eq!(0, app.editor.tree.views().count())),
            ),
        ],
        true,
    )
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn space_matches_line_endings() -> anyhow::Result<()> {
    test_with_config(
        AppBuilder::new().with_file("foo.txt", None),
        (
            indoc! {"\
                #[|]#one two
                three four
            "},
            "gwo ",
            indoc! {"\
                one tw#[o|]#
                three four
            "},
        ),
    )
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn more_than_twenty_six_matches_use_uppercase_labels() -> anyhow::Result<()> {
    let mut app = AppBuilder::new()
        .with_input_text(format!("#[|]#{}", "vi\n".repeat(30)))
        .build()?;

    test_key_sequence(
        &mut app,
        Some("gwviA"),
        Some(&|app| {
            let (view, doc) = current_ref!(app.editor);
            assert_eq!(Mode::Normal, app.editor.mode());
            let selection = doc.selection(view.id).primary();
            assert_eq!((78, 79), (selection.anchor, selection.head));
        }),
        false,
    )
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn more_than_fifty_two_matches_fall_back_to_two_character_labels() -> anyhow::Result<()> {
    let mut app = AppBuilder::new()
        .with_input_text(format!("#[|]#x\n{}", "vi\n".repeat(60)))
        .build()?;

    test_key_sequence(
        &mut app,
        Some("gwviaa"),
        Some(&|app| {
            let (view, doc) = current_ref!(app.editor);
            assert_eq!(Mode::Normal, app.editor.mode());
            let selection = doc.selection(view.id).primary();
            assert_eq!((2, 3), (selection.anchor, selection.head));
        }),
        false,
    )
    .await?;

    Ok(())
}
