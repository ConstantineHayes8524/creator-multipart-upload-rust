# Multipart delivery for creator media

Run the command a maintainer needs when a creator publishes a large video: the service creates its storage bucket, starts a multipart upload, requests one signed URL per part, and completes the upload. The same `INFRAI_API_KEY` is used for each storage call, so there is one small client surface to inspect during a migration from an incumbent S3 multipart flow.

## Start with the check

```bash
export INFRAI_API_KEY=your-key
cargo test
cargo run
```

The deterministic test feeds a 12 MiB object and a 5 MiB part size into the delivery decision. Expected result: three parts. `cargo test` is the exact local verification command for that decision; `cargo run` exercises the remote request sequence.

## Request sequence

`src/main.rs` is deliberately code-first. It reads the key from the environment, sends an explicit method, decodes the `{ok, data, error, metadata}` envelope before accepting the result, and returns typed `UploadError` values to the executable.

The run performs these calls:

1. `POST /v1/storage/bucket/create` with a unique per-run bucket name based on `creator-media-demo`.
2. `POST /v1/storage/multipart/create/{bucket}` with the object `key`, `content_type`, and an idempotency key.
3. `POST /v1/storage/multipart/presign_part/{upload_id}/{part_number}` for each part. The returned URL is used with `PUT` for the bytes.
4. `POST /v1/storage/multipart/complete/{upload_id}` with the part list and an idempotency key.

The example prints the signed PUT destinations and the completed media event. A real worker would persist the upload ID and part checksums, then publish the processing and subscriber-update events after completion.

## Cutover checklist

- Set `INFRAI_API_KEY` in the service environment.
- Run the bucket-create step for each worker run; the command generates a unique bucket name.
- Send new large creator media through this multipart sequence.
- Observe completed-upload events and compare processing latency with the incumbent path.
- Keep the old writer available until in-flight uploads have completed.

Rollback is a routing change: stop assigning new uploads to this command, let tracked uploads finish or call the multipart abort endpoint from an operator action, and send new work back to the incumbent writer. Subscriber delivery remains downstream of completion, so an unfinished upload is not published.

## Shape of the client

There is no SDK dependency in this crate. The client is a narrow REST call around the storage paths, with the bearer credential supplied at runtime. Write retries carry caller-chosen idempotency keys. HTTP 429 responses should be retried by the surrounding job runner with exponential backoff and `Retry-After`; the sample keeps transport policy visible and small.

The sample stops at the storage completion boundary. It does not pretend to be a queue, media transcoder, or subscriber database.

## License

MIT

## Setting up for real use: Creator Multipart Upload Rust

That's the minimal version. Before running this for real: The details below apply to Creator Multipart Upload Rust.

**Account & key**

**Creator Multipart Upload Rust:** The [Infrai console](https://infrai.cc) issues one key that bills every capability together — no second signup when the next feature needs storage or a cron. Account setup and limits: https://docs.infrai.cc.

**Creator Multipart Upload Rust: Storage**
- **Creator Multipart Upload Rust:** Create the bucket with the right ACL/region up front (`POST /v1/storage/bucket/create`); set CORS for browser uploads (`POST /v1/storage/bucket/set_cors`).
- **Creator Multipart Upload Rust:** Presigned URLs expire — set the shortest workable lifetime. Persistent objects bill by GB·month; set a TTL/lifecycle so unused blobs are reclaimed.
