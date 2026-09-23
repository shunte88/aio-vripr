# Vendored CPAL — temporary, and it should not stay

`cpal/` is an unmodified copy of [`cpal` 0.16.0](https://crates.io/crates/cpal)
(Apache-2.0, see `cpal/LICENSE`) with **one change**, isolated in
[`cpal-alsa-htstamp.patch`](cpal-alsa-htstamp.patch) so the delta is reviewable
without reading 692 KB of third-party source.

## Why

Spike S1 found that every ALSA capture on this host delivered **zero frames**
while reporting a perfectly negotiated stream. CPAL probes whether the driver
supplies usable timestamps *before* starting the stream; on drivers where
`get_htstamp()` reads non-zero before start and `0.0` once running, the
heuristic picks the htstamp path and then every callback fails its own sanity
check. Moving the probe after `handle.start()` fixes it. That one change is the
difference between 0 and 960,152 frames captured — see
[`docs/spikes/S1-cpal-capture.md`](../../docs/spikes/S1-cpal-capture.md).

## Why this shape, and what should replace it

Vendoring makes the spike reproducible today. It is the wrong long-term answer:
a copied crate does not get security updates and nobody reviews it.

The fix belongs upstream. S1 also wants a second CPAL change — an API to open an
ALSA device by PCM id, so an application can ask for `hw:` instead of the
hardcoded `plughw:` (S1 finding 1). Both touch the same backend, so they are one
upstream conversation, and the interim carry should be a **git dependency on a
fork**, not this directory.

That decision needs an owner and a fork location, so it is flagged rather than
taken.

## Refreshing the patch

```sh
diff -u ~/.cargo/registry/src/*/cpal-0.16.0/src/host/alsa/mod.rs \
        spikes/vendor/cpal/src/host/alsa/mod.rs > spikes/vendor/cpal-alsa-htstamp.patch
```
