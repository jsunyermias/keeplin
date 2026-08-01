# Pull-request governance checks

`check-review-governance.js` enforces the two permitted paths through the pull-request
template when a pull request leaves draft state:

- an independent reviewer, a distinct implementer, both completed review assertions and a
  GitHub evidence link; or
- every maintainer-waiver field, a change to `docs/review-debt.md`, and an entry that names
  the exact pull request.

The script is intentionally dependency-free so the workflow can load it from
`actions/github-script`. Run its regression suite locally with:

```sh
node --test .github/scripts/check-review-governance.test.js
```

Draft pull requests remain exempt so authors can prepare the body incrementally. Editing the
body or marking the pull request ready triggers CI again.
