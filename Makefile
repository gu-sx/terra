.PHONY: check fmt

# The checks CI runs. Clippy runs twice so that both the default build and
# every feature (such as terra-core's `testing`) are checked; the tests run
# with every feature so that none hides behind a flag.
check:
	cargo fmt --all --check
	cargo clippy --workspace --all-targets --locked -- -D warnings
	cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
	cargo test --workspace --all-features --locked

fmt:
	cargo fmt --all
