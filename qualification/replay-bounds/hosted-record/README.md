# Hosted replay-bounds validation record

These files are the verified artifacts from [run 40046104026](https://github.com/EricSpencer00/Resilient/actions/runs/40046104026), retained beyond GitHub artifact expiry. Baseline and final folders contain each of the four CLI case records; attestations contains the three paired failure-to-success records. The valid-response control passes on both revisions.

`source-identity.json` identifies the tested PR merge revision and its tree. To reconstruct it, check out the baseline revision in a disposable checkout, apply `validated-source.patch` with `git apply --index`, and compare `git write-tree` with `validated_tree`. No build is needed for that identity check. Build and test reruns use the hosted Replay qualification workflow.

The attestations bind command, declared test input, toolchain and recorded outcomes. They do not establish source-to-query correctness, complete tool qualification or assessor approval. See the parent scope document and the recorded manifests for exact operational context.
