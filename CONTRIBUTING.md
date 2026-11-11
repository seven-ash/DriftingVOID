# Contributing

Thank you for wanting to help with Drifting VOID.

## Licence of contributions

Drifting VOID is licensed under the GNU General Public License, version 3
or (at your option) any later version, with additional terms on
attribution, marking of modified versions and the program's name (see
`LICENSE` and `NOTICE`). By contributing, you license your contribution
under the same licence and the same additional terms. You keep the
copyright to your contribution.

One file is different: `src/lsd.rs` is a port of AGPL-licensed reference
code, so a contribution to that file is licensed under the GNU Affero
General Public License, version 3 or later (`LICENSE-AGPL`).

## Sign-off (Developer Certificate of Origin)

Every commit in a pull request must be signed off: a line at the end of
the commit message stating that you wrote the change, or otherwise have
the right to submit it, under the project's licence.

    Signed-off-by: Your Name <you@example.com>

`git commit -s` adds the line. A real name or a stable, public handle
is fine; the address must reach you.

By signing off you certify the following:

```
Developer Certificate of Origin
Version 1.1

Copyright (C) 2004, 2006 The Linux Foundation and its contributors.

Everyone is permitted to copy and distribute verbatim copies of this
license document, but changing it is not allowed.


Developer's Certificate of Origin 1.1

By making a contribution to this project, I certify that:

(a) The contribution was created in whole or in part by me and I
    have the right to submit it under the open source license
    indicated in the file; or

(b) The contribution is based upon previous work that, to the best
    of my knowledge, is covered under an appropriate open source
    license and I have the right under that license to submit that
    work with modifications, whether created in whole or in part
    by me, under the same open source license (unless I am
    permitted to submit under a different license), as indicated
    in the file; or

(c) The contribution was provided directly to me by some other
    person who certified (a), (b) or (c) and I have not modified
    it.

(d) I understand and agree that this project and the contribution
    are public and that a record of the contribution (including all
    personal information I submit with it, including my sign-off) is
    maintained indefinitely and may be redistributed consistent with
    this project or the open source license(s) involved.
```

## How pull requests are taken in

The maintainer reviews each pull request. A change may be applied as
submitted or reworked before it is merged. When it is reworked, the
commit keeps the contributor's authorship and sign-off (as
`Co-authored-by:` and `Signed-off-by:` lines), so the record of who
contributed stays in the history.

Code copied from elsewhere must come with its licence, and that licence
must be compatible with the GNU GPL version 3; say where it came from in
the pull request.
