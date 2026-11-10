.PHONY: check fmt

# The checks CI runs.
check:
	cargo fmt --all --check
	cargo clippy --workspace --all-targets --locked -- -D warnings
	cargo test --workspace --locked

fmt:
	cargo fmt --all
