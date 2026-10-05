# miao Remote Control integration

The transport layer is `mtty_ui::remote_control`. It invokes a trusted miao
executable directly with `runtime access`, writes one versioned JSON request
to a private stdin pipe, closes stdin, and validates one stdout response. It
does not type into a terminal pane, start a Runtime, or read/cache the Runtime's
administrator credential. This requires a miao build with the `runtime access`
command.

Call it from a background job with an explicit cancellation flag. Requests
support status, session context, invitation issuance, pending device keys,
exact-key approval, invitation rejection, device listing, observed-version
revocation, and relay setup. Every operation after status requires the observed
Runtime ID. Supply the selected database when it differs from the default;
never substitute the default Runtime for an explicitly attached remote pane.

The request is limited to 64 KiB and response to 1 MiB. The subprocess has a
120-second budget, including the multiple individually bounded HTTP requests
made during relay setup. Cancellation or timeout kills and reaps the owned
child; an uncertain mutation returns `Unconfirmed`. The caller must inspect
current pairing/device/relay status before another action, and must never
retry automatically. A successful response from a different bound Runtime is
rejected. Raw stderr and unrecognized provider errors are not surfaced.

Requests can contain relay passwords and replies can contain pairing secrets.
Do not log, persist or debug-format them. The transport types deliberately do
not implement `Debug` for secret-bearing values. Use fixed translated messages
for the `Error` variants in the interface.

The connection window, QR rendering, explicit pane Runtime/database reporting,
owner confirmation of candidate keys/scopes and account setup forms are separate
implementation work tracked in #74. The transport alone does not enable those
user-facing flows.
