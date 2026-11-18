# folder structure

- docs/specifications.md
	maintained by human software architect, edit only if explicitely asked
- docs/implementation.md
	current user confirmed choices, that do not appear in [specifications.md]. maintained by claude, and should always kept consistent with [specifications.md]
	it shouldn't restate what specifications.md already imposes
- docs/journal.md
	time stamped list of issues, workarounds, decision changes over the development. maintained by claude

# workflow

the user is making the commits and pushing, claude doesn't unless being asked to

# safety

avoid anything that can break the real phone
