# Disposal authorization directives

Use this procedure when a review-ledger finding is `resolved` or `dismissed`, when a previously
reified finding must become `advisory`, when `GENESIS` must be authenticated, or when an authorized
tombstone is required. The evaluator accepts the pull-request author's own directive only while
its exhaustive collaborator enumeration finds no principal other than the repository owner.

1. Confirm the exact target ID and state. Ordinary IDs use `F-` followed by at least three digits;
   the synthetic anchor uses `GENESIS`. The target state is `resolved`, `dismissed`, `advisory`,
   `genesis`, or `tombstone` as appropriate to that path.
2. Write a specific, non-empty reason. For `resolved`, identify the mechanical check that now
   passes. For `dismissed`, explain the priority decision or cite the accepted ADR. Do not use a
   generic approval phrase.
3. Build one single-line marker, replacing the example values with JSON strings for the exact
   target:

   ```text
   <!-- keeplin-review-loop-authorize {"finding":"F-123","state":"dismissed","reason":"Accepted by ADR 0015"} -->
   ```

4. Post that marker in a comment or review on the same pull request. The author must have
   `OWNER`, `MEMBER`, or `COLLABORATOR` association. A dismissed review is not valid evidence.
5. Copy the comment or review numeric ID, its author login, and the SHA-256 digest of its complete
   body into the ledger row's Resolution JSON as `referenceId`, `author`, and `bodyDigest`. Preserve
   any resolution proof fields required by the target state. For example:

   ```json
   {"referenceId":123456789,"author":"jsunyermias","bodyDigest":"64 lowercase hexadecimal characters"}
   ```

6. For an ordinary finding whose disposition was reopened or changed, issue the directive after
   that boundary. GitHub timestamps have one-second granularity, so a directive in the same second
   is rejected; post a fresh directive in a later second.
7. Trigger a new CI evaluation by pushing a commit or editing the pull-request body. Updating the
   ledger means editing that body and therefore triggers the same evaluation.
   Do not add a secret: the default-branch evaluator uses its existing `GITHUB_TOKEN` to enumerate
   collaborators with `affiliation=all` and follows every `Link: rel="next"` page.
8. Inspect the authoritative `Review loop converged` check. A `403`, rate limit, transport error,
   malformed response, or pagination sequence that cannot be proven exhausted makes membership
   unknown and refuses the disposition. Do not reinterpret that result as an empty or
   single-principal repository; retry after the API failure is resolved.
9. Confirm the next App-authored journal observation records the same reference ID, author, and
   body digest. For `resolved`, also confirm the named required check is successful on the exact
   evaluated head and workflow run. The journal entry is the durable record of the authorization.

If another principal appears in the exhaustive enumeration, owner self-authorization lapses
automatically. Obtain the directive from a different qualifying principal. If the repository is
transferred to an organization, stop relying on this procedure and revisit ADR 0015 because the
collaborators endpoint no longer proves the full qualifying population described there.
