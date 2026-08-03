# `scripts/tests/test_bounded_history.py` — bounded-history visible-prose regressions

## What is tested

`scripts/check-bounded-history.py` requires three manually enrolled surfaces to carry one
canonical sentence verbatim in prose selected by a declared Markdown subset. The tests execute
the real checker against temporary fixture roots and assert its process exit status.

| Test | Expected | What it pins |
|------|----------|--------------|
| `test_the_real_repository_passes` | `0` | all checked-in surfaces satisfy the rule |
| `test_intact_fixtures_pass` | `0` | the fixture baseline is valid |
| `test_line_wrapping_does_not_matter` | `0` | collapsed prose whitespace makes wrapping harmless |
| `test_an_html_comment_does_not_satisfy_the_check` | `1` | comment metadata is not a policy statement |
| `test_a_fenced_code_block_does_not_satisfy_the_check` | `1` | fenced examples do not count |
| `test_a_tilde_fenced_code_block_does_not_satisfy_the_check` | `1` | tilde fences do not count |
| `test_a_fence_with_three_leading_spaces_does_not_satisfy_the_check` | `1` | up to three spaces may indent a fence |
| `test_an_indented_code_block_does_not_satisfy_the_check` | `1` | blank-line-delimited four-space code does not count |
| `test_an_indented_code_block_at_document_start_does_not_satisfy_the_check` | `1` | document-start indentation is code without a preceding blank line |
| `test_a_tab_indented_code_block_does_not_satisfy_the_check` | `1` | tab-indented code does not count |
| `test_a_fence_inside_one_blockquote_does_not_satisfy_the_check` | `1` | one quoted fence level is recognized |
| `test_a_blockquote_fence_cannot_close_a_margin_fence` | `1` | a closer must match its opener's quote level |
| `test_a_margin_fence_cannot_close_a_blockquote_fence` | `1` | quote-level matching is symmetric |
| `test_an_unused_link_reference_definition_does_not_satisfy_the_check` | `1` | a non-rendered single-line definition does not count |
| `test_a_fence_inside_an_html_comment_does_not_hide_later_prose` | `0` | a comment cannot open a phantom fence |
| `test_a_multiline_html_comment_does_not_satisfy_the_check` | `1` | hidden comment content does not count |
| `test_a_longer_closing_fence_exposes_later_prose` | `0` | a closer at least as long as its opener closes the fence |
| `test_an_html_comment_marker_inside_inline_code_does_not_hide_later_prose` | `0` | inline code cannot open an HTML comment |
| `test_same_line_inline_code_does_not_satisfy_the_check` | `1` | hidden inline-code content does not count |
| `test_a_shorter_fence_does_not_close_a_longer_fence` | `1` | a shorter run is not a closer |
| `test_a_different_fence_marker_does_not_close_a_fence` | `1` | backticks and tildes cannot close each other |
| `test_normal_prose_still_satisfies_the_check` | `0` | ordinary prose remains positive |
| `test_a_glossary_of_the_words_does_not_satisfy_the_check` | `1` | vocabulary alone is not the canonical statement |
| `test_a_weakened_sentence_does_not_satisfy_the_check` | `1` | hedging the limitation fails |
| `test_dropping_the_consequence_does_not_satisfy_the_check` | `1` | omitting the cost of truncation fails |
| `test_a_missing_surface_fails_closed` | `1` | absence is never skipped |
| `test_every_surface_is_checked_independently` | `1` | no enrolled surface rides on another |

## Declared-subset boundary tests

The implementation and its companion name six constructs outside the grammar. One test per
construct pins the current behavior rather than implying parser support:

| Test | Current status | Unhandled construct |
|------|----------------|---------------------|
| `test_deeper_blockquote_fences_pin_out_of_subset_behavior` | `0` | blockquotes deeper than one level |
| `test_list_nested_fences_pin_out_of_subset_behavior` | `0` | list continuation/nested list code |
| `test_raw_html_blocks_pin_out_of_subset_behavior` | `0` | raw HTML other than comments |
| `test_multiline_reference_definitions_pin_out_of_subset_behavior` | `0` | split reference definitions |
| `test_multiline_inline_code_spans_pin_out_of_subset_behavior` | `0` | multiline inline code |
| `test_inline_link_titles_pin_out_of_subset_behavior` | `0` | inline link/image destinations and titles |

In these representatives the raw canonical bytes currently count, so the checker returns `0`.
That result is a stability pin only: the public contract remains that a sentence hidden in an
unsupported construct is neither guaranteed to count nor guaranteed to be ignored.

## Harness and old-implementation measurement

Each fixture contains all three enrolled paths. `BOUNDED_HISTORY_CHECK` can point the same tests
at a scratch executable, which is how the committed awk implementation is measured without
altering the working tree. With no override the suite runs the checked-in Python implementation.

## Run

```sh
python3 -m unittest discover -s scripts/tests -p 'test_*.py'
```

No network, API, Rust toolchain or model is involved.

## Related files

- `scripts/check-bounded-history.py` — checker under test.
- `scripts/companion_tool.py` — shared Markdown-subset implementation.
- `scripts/check-docs.sh` — invokes the checker as check 12.
