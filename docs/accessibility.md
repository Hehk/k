# macOS accessibility: blocked

Kestrel's custom repository and diff content does not yet have a working
accessibility bridge. Native window controls, keyboard shortcuts, tooltips, and
debug selectors do not provide that contract. **[Issue #14](https://github.com/Hehk/k/issues/14)
remains open; this document is an investigation, not an accessibility fix.**

## Confirmed framework boundary

The workspace pins GPUI **0.2.2**. These paths refer to that crate's source:

| Boundary | Evidence |
| --- | --- |
| Custom elements | `src/element.rs`: `Element` provides layout, prepaint, and paint, but no accessibility-node interface. |
| Platform publication | `src/platform.rs`: the private `PlatformWindow` trait has no semantic-tree update or accessibility-action callback. |
| macOS content view | `src/platform/mac/window.rs`: `build_classes` registers `GPUIView`, an `NSView` subclass, with painting, input, and `NSTextInputClient` methods. It does not publish custom accessibility children, actions, focus, or notifications. Inherited AppKit behavior is not an application content tree. |
| Native escape hatch | `src/window.rs`: `Window` implements `raw_window_handle::HasWindowHandle`. `MacWindow` returns its `native_view` as an AppKit handle. This is a possible attachment point, not an accessibility adapter. |
| Headless verification | `src/platform/test/window.rs`: `TestWindow::window_handle` is unimplemented because test windows have no native backing. GPUI component tests cannot verify the AppKit tree. |

In Kestrel, `controls.rs` builds painted divs; `top_bar/navigation.rs` adds
shortcuts and tooltips. `text_input.rs` registers an `ElementInputHandler` for
native text input, not accessible text-field values and selection.
`comparison_view.rs` virtualizes diff rows, with custom selection and a painted
gutter. None of these paths publishes a native semantic tree. Appearance's
accessibility-preference observer only handles visual preferences.

## Integration decision still required

There is no supported node API to turn on in the pinned dependency. A newer GPUI
release must be checked for an actual macOS bridge before considering an upgrade;
a version bump alone is not evidence of support.

A direct AppKit bridge using the native view handle is technically possible.
It would need accessible elements attached to the content view, with native
queries and actions connected to the existing controls and model. An adapter
such as AccessKit is another candidate, not a verified integration. Neither
approach has been implemented or validated here. A GPUI fork is not assumed to
be necessary, and no framework has been vendored.

Before adopting either route, prove a real control's native discovery and
activation, then text value/selection and a virtualized list. Resolve:

- Per-window element ownership, teardown, and stale callbacks after route/tab
  changes; no global class swizzling or replacement of GPUI's input view.
- Stable node identities and frame-consistent bounds, clipping, hit testing, and
  screen-coordinate conversion, including scrolling and detached windows.
- Main-thread action dispatch without reentrant mutable GPUI access. Native
  actions must reuse application actions and enforce current enabled state.
- Bidirectional focus and selection, with change notifications after publication.
  Loading and errors must be announced without repeated per-frame announcements.
- Logical list items and readable diff text beyond painted rows, including source
  line identity, additions/deletions, selected text, and Unicode range conversion.
  Exposing only the currently painted row labels is insufficient.

This is framework/native integration work, not a missing-label patch. Adding
metadata without this bridge would leave the reported workflow inaccessible.

## Acceptance gate

Use a disposable repository and isolated configuration, never a real saved
workspace. Native checks require an explicitly authorized session; do not enable
Accessibility permissions, VoiceOver, or Voice Control automatically.

Record the build revision, macOS version, fixture, observed accessibility tree,
actions, focus/selection notifications, and actual assistive-technology outcomes:

1. On Home, identify and activate Open Repository; operate the native picker and
   return to the application with correct focus.
2. Identify the repository and branch search field; read/edit its value and
   selection. Navigate filtered branches, identify the selected row, and open it.
3. Identify and choose a base, read loading/error states, and retry failures.
4. Navigate changed files and read diff content, including offscreen lines and
   additions/deletions. Read and copy selected text without editing the repository.
5. Identify selected tabs and enabled/disabled Back/Forward; operate Recently
   viewed, dismiss it, and confirm focus restoration and preserved comparison pins.
6. Repeat across two windows and tab closure/detachment to detect stale nodes or
   actions targeting the wrong view.

Inspect actual roles, names, values, relationships, selected/disabled states,
focus, and supported actions—not just strings. Complete the repository-to-diff
journey with **VoiceOver** and verify named-control operation with **Voice Control**.
Native tree inspection is necessary but does not replace those workflows.

No native acceptance result is recorded here. The bridge and the assistive-
technology checks remain outstanding. Passing Rust/component tests cannot close
this issue.
