# # 使用较新的 Rust 版本作为构建阶段
# FROM rust:1.77-alpine as builder
# # 安装构建必要的依赖
# RUN apk add --no-cache musl-dev

# WORKDIR /usr/src/app
# COPY . .
# RUN cargo build --release

# # 运行阶段
# FROM alpine:latest
# COPY --from=builder /usr/src/app/target/release/my_wasm_project /usr/local/bin/my-app
# CMD ["my-app"]

FROM rust:1.68.2
# WORKDIR /usr/src/app
COPY . /
RUN cargo install --path .
ENTRYPOINT ["my-app"]
