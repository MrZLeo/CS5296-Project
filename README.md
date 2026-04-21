# CS5296 Project

本项目用于比较同一组微基准工作负载在三种本地运行路径上的表现：

- `wasmedge-wasm`：直接执行普通 Wasm 模块
- `wasmedge-aot`：执行 WasmEdge AOT 编译后的工件
- `docker`：在 Docker 容器中执行相同应用

当前仓库的主实验目标是观察：

- 端到端冷启动时延
- 应用内部纯计算时间
- 启动开销和计算开销的分离
- 普通 Wasm 与 AOT Wasm 的差异

## 软件架构

### 总体结构

```text
                +----------------------+
                |   bench_local.rs     |
                | 宿主侧基准驱动器      |
                +----------+-----------+
                           |
        +------------------+------------------+
        |                  |                  |
        v                  v                  v
  wasmedge my-app.wasm  wasmedge my-app-aot.wasm  docker run my-docker-app:latest
        |                  |                  |
        +------------------+------------------+
                           |
                           v
                    +-------------+
                    |   my-app    |
                    | src/main.rs |
                    +------+------+
                           |
                           v
                    +-------------+
                    |  src/lib.rs |
                    | workload库   |
                    +-------------+
```

### 模块职责

#### 1. 工作负载库

文件：[src/lib.rs](src/lib.rs)

负责：

- 定义统一工作负载接口 `Workload`
- 解析应用 CLI 参数
- 执行具体 workload
- 生成统一 JSON 输出
- 提供基准驱动解析结果所需的公共函数

当前支持 3 类 workload：

- `noop`
  作用：尽量逼近“只有启动，没有业务计算”的场景
- `fib`
  作用：CPU-bound 计算，使用递归 Fibonacci
- `alloc_touch`
  作用：模拟初始化和内存触达开销，按页写入大块内存

#### 2. 应用入口

文件：[src/main.rs](src/main.rs)

职责很简单：

- 调用 `parse_app_args`
- 执行 workload
- 输出单行 JSON

标准输出格式如下：

```json
{"workload":"fib","parameter":"n=40","result_digest":102334155,"internal_compute_ms":141.444000}
```

字段含义：

- `workload`：工作负载类型
- `parameter`：该工作负载的参数
- `result_digest`：结果摘要，用于确认不同运行路径计算结果一致
- `internal_compute_ms`：应用进程内部纯计算时间，不含宿主侧启动时间

#### 3. 本地基准驱动

文件：[src/bin/bench_local.rs](src/bin/bench_local.rs)

这是当前实验的核心组件。它运行在宿主机侧，负责：

- 逐个拉起不同 runtime
- 顺序采样，避免并发干扰
- 记录从宿主机发起进程到子进程退出的端到端时间 `e2e_ms`
- 解析应用输出中的 `internal_compute_ms`
- 计算 `startup_overhead_ms = e2e_ms - internal_compute_ms`
- 生成逐样本 CSV 和汇总统计

当前比较的 runtime：

- `wasmedge-wasm`
- `wasmedge-aot`
- `docker`

#### 4. 容器封装

文件：[Dockerfile](Dockerfile)

职责：

- 构建 Docker 镜像 `my-docker-app:latest`
- 通过 `ENTRYPOINT ["my-app"]` 透传 workload 参数

#### 5. 测试脚本

文件：[scale_test.sh](scale_test.sh)

这是一个快捷入口，本质上只是包装：

```bash
cargo run --bin bench_local --release -- "$@"
```

## 目录说明

```text
.
├── pyproject.toml               # uv 管理的 Python 绘图依赖
├── scripts/
│   └── plot_benchmarks.py       # 从 CSV 生成图表
├── src/
│   ├── lib.rs                  # workload 库和 JSON 输出逻辑
│   ├── main.rs                 # 应用入口
│   └── bin/
│       └── bench_local.rs      # 宿主侧本地基准驱动
├── Dockerfile                  # Docker 运行路径
├── scale_test.sh               # 基准快捷脚本
├── my-app-aot.wasm             # WasmEdge AOT 工件
├── docker-app.yaml             # K8s Docker Pod 示例
├── wasm-app.yaml               # K8s Wasm Pod 示例
└── runtime.yaml                # K8s RuntimeClass 示例
```

## 当前测试方法

### 测试目标

当前主测试是“本地 runtime 级对比”，不是 K8s Pod 级对比。

也就是说，当前标准流程比较的是：

- `wasmedge my-app.wasm`
- `wasmedge my-app-aot.wasm`
- `docker run --rm my-docker-app:latest`

而不是：

- Kubernetes 调度开销
- 镜像拉取时间
- 网络服务链路

### 冷启动定义

本项目目前将“冷启动”定义为：

- 每个样本都新启动一个 Wasm 实例或一个 Docker 容器
- 所有本地工件已经存在
- 不把镜像拉取和远程下载算入实验结果

因此这里的冷启动主要表示：

- 运行时装载
- 实例初始化
- 应用启动
- 应用执行并退出

### 计时口径

对于每个样本，采集 3 个核心量：

- `e2e_ms`
  含义：宿主机从拉起命令到子进程退出的总时间
- `internal_compute_ms`
  含义：应用内部 workload 真正执行的时间
- `startup_overhead_ms`
  含义：`e2e_ms - internal_compute_ms`

这个拆分的好处是：

- 可以区分“启动慢”还是“计算慢”
- 可以直接看到普通 Wasm 与 AOT 的差异主要来自执行阶段还是启动阶段
- 可以避免把 workload 本身耗时误判成启动时延

### 工作负载设计

#### `noop`

意义：

- 估计最接近纯启动成本
- 观察 runtime 最小启动开销

#### `fib`

默认参数：`n=40` 用于 benchmark，应用默认值是 `n=45`

意义：

- 强 CPU-bound
- 适合观察普通 Wasm 与 AOT 在纯计算上的差距

#### `alloc_touch`

默认参数：`64 MiB`

意义：

- 模拟较重初始化与内存页触达
- 观察 runtime 对内存相关 workload 的影响

## 软件依赖与环境准备

当前主流程建议具备以下工具：

- Rust toolchain
- Cargo
- `uv`
- `wasm32-wasip1` target
- WasmEdge
- Docker

可选：

- `kubectl`
- 支持 `wasmedge` 的 Kubernetes RuntimeClass

### 1. 检查 Rust

```bash
rustc --version
cargo --version
```

### 2. 安装 Wasm target

```bash
rustup target add wasm32-wasip1
```

### 3. 检查 WasmEdge

```bash
wasmedge --version
wasmedge compile --help
```

### 4. 检查 Docker

```bash
docker version
```

### 5. 检查 uv

```bash
uv --version
```

## 构建步骤

建议每次修改 workload 或应用逻辑后，按下面顺序重建。

### 步骤 1：运行单元测试

```bash
cargo test
```

### 步骤 2：构建本机二进制

```bash
cargo build --release --bins
```

会生成：

- `target/release/my-app`
- `target/release/bench_local`

### 步骤 3：构建普通 Wasm 工件

```bash
cargo build --target wasm32-wasip1 --release
```

会生成：

- `target/wasm32-wasip1/release/my-app.wasm`

### 步骤 4：生成 AOT 工件

```bash
wasmedge compile target/wasm32-wasip1/release/my-app.wasm my-app-aot.wasm
```

说明：

- 仓库当前默认 AOT 工件名是 `my-app-aot.wasm`
- 虽然扩展名仍然是 `.wasm`，但它实际上是给 WasmEdge 使用的 AOT 编译产物
- 每次更新应用逻辑后，都应该重新执行这一步

### 步骤 5：构建 Docker 镜像

```bash
docker build -t my-docker-app:latest .
```

## 单独运行各条路径

### 1. 直接运行本机版本

```bash
target/release/my-app --workload noop
target/release/my-app --workload fib --n 40
target/release/my-app --workload alloc_touch --bytes 67108864
```

### 2. 运行普通 Wasm

```bash
wasmedge target/wasm32-wasip1/release/my-app.wasm --workload noop
wasmedge target/wasm32-wasip1/release/my-app.wasm --workload fib --n 40
wasmedge target/wasm32-wasip1/release/my-app.wasm --workload alloc_touch --bytes 67108864
```

### 3. 运行 AOT Wasm

```bash
wasmedge my-app-aot.wasm --workload noop
wasmedge my-app-aot.wasm --workload fib --n 40
wasmedge my-app-aot.wasm --workload alloc_touch --bytes 67108864
```

### 4. 运行 Docker

```bash
docker run --rm my-docker-app:latest --workload noop
docker run --rm my-docker-app:latest --workload fib --n 40
docker run --rm my-docker-app:latest --workload alloc_touch --bytes 67108864
```

## 基准测试指令与步骤

### 快速验证

先跑一个样本，确认三条路径都正常：

```bash
./scale_test.sh --samples 1
```

### 标准实验

默认建议每种 workload、每种 runtime 采样 30 次：

```bash
./scale_test.sh --samples 30
```

这会按以下顺序执行：

1. `wasmedge-wasm`
2. `wasmedge-aot`
3. `docker`

每个 runtime 都会依次跑：

1. `noop`
2. `fib(n=40)`
3. `alloc_touch(bytes=67108864)`

### 直接使用基准驱动

如果不想通过脚本，也可以直接运行：

```bash
cargo run --bin bench_local --release -- --samples 30
```

### 自定义工件路径

如果你想换工件或输出文件，可以这样运行：

```bash
cargo run --bin bench_local --release -- \
  --samples 30 \
  --wasm-artifact target/wasm32-wasip1/release/my-app.wasm \
  --wasm-aot-artifact my-app-aot.wasm \
  --docker-image my-docker-app:latest \
  --output target/bench-results/local-bench.csv \
  --summary-output target/bench-results/summary.csv
```

## 输出结果说明

基准运行会输出两部分内容。

### 1. 逐样本实时日志

示例：

```text
[1/1] wasmedge-aot fib n=40 e2e=151.540ms internal=141.444ms startup=10.096ms
```

字段含义：

- `wasmedge-aot`：当前 runtime
- `fib`：当前 workload
- `n=40`：参数
- `e2e`：端到端时间
- `internal`：应用内计算时间
- `startup`：推导出来的启动开销

### 2. 汇总统计表

示例表头：

```text
runtime    workload       parameter      samples       mean        p50        p95     stddev startup_mean
```

含义：

- `mean`：端到端时间均值
- `p50`：端到端中位数
- `p95`：端到端 95 分位
- `stddev`：端到端标准差
- `startup_mean`：启动开销均值

汇总表会同时写入：

```text
target/bench-results/summary.csv
```

CSV 字段：

```text
runtime,workload,parameter,samples,mean,p50,p95,stddev,startup_mean
```

### 3. 明细 CSV 文件

默认输出路径：

```text
target/bench-results/local-bench.csv
```

CSV 字段：

```text
runtime,workload,parameter,sample,e2e_ms,internal_compute_ms,startup_overhead_ms,exit_code
```

这个文件适合后续：

- 导入 Excel
- 画箱线图/柱状图
- 做 runtime 间统计比较

## 使用 uv 生成测试图

项目使用 `uv` 管理 Python 运行环境和 `matplotlib` 依赖。

### 1. 安装绘图依赖

推荐把 uv 缓存放在仓库内，避免污染全局缓存：

```bash
UV_CACHE_DIR=.uv-cache uv sync
```

### 2. 运行绘图脚本

```bash
UV_CACHE_DIR=.uv-cache uv run python scripts/plot_benchmarks.py
```

### 3. 自定义输入输出路径

```bash
UV_CACHE_DIR=.uv-cache uv run python scripts/plot_benchmarks.py \
  --detail-csv target/bench-results/local-bench.csv \
  --summary-csv target/bench-results/summary.csv \
  --output-dir target/bench-results/plots
```

### 4. 默认输出目录

```text
target/bench-results/plots
```

脚本当前会生成：

- `e2e_overview.pdf`
  对比各 runtime 在各 workload 上的 `mean`、`p50`、`p95` 端到端时延
- `startup_overview.pdf`
  对比各 runtime 在各 workload 上的 `mean`、`p50`、`p95` 启动开销
- `breakdown.pdf`
  展示各 workload 下 `startup_overhead_ms` 与 `internal_compute_ms` 的平均拆分

说明：

- 当前脚本只输出 `*.pdf`，不会生成 `*.png`
- `--detail-csv` 是实际绘图输入；`--summary-csv` 目前主要保留为 CLI 兼容参数
- 脚本启动时会清理旧版命名的图表文件
- 建议先执行 `./scale_test.sh --samples 30`，再生成图表

## 推荐的完整复现实验流程

从零开始，建议严格按下面顺序执行：

```bash
rustup target add wasm32-wasip1
cargo test
cargo build --release --bins
cargo build --target wasm32-wasip1 --release
wasmedge compile target/wasm32-wasip1/release/my-app.wasm my-app-aot.wasm
docker build -t my-docker-app:latest .
./scale_test.sh --samples 30
```

## K8s 配置说明

仓库中还有 3 个 Kubernetes 相关文件：

- [runtime.yaml](runtime.yaml)
- [wasm-app.yaml](wasm-app.yaml)
- [docker-app.yaml](docker-app.yaml)

它们的用途是：

- `runtime.yaml`
  定义 `RuntimeClass`，名称为 `wasmedge`
- `docker-app.yaml`
  给 Docker 镜像提供一个 Pod 示例
- `wasm-app.yaml`
  给 Wasm RuntimeClass 提供一个 Pod 示例

但要注意：

- 当前 README 的主测试流程不依赖 Kubernetes
- 当前标准 benchmark 结果来自本地 `bench_local`
- `wasm-app.yaml` 目前仍指向示例镜像 `secondstate/rust-learning:helloworld`，不是当前仓库构建出的工件

因此，K8s 文件目前更适合作为后续扩展实验的参考配置，而不是当前主实验路径。

## 常见问题

### 1. `Docker preflight failed`

通常表示：

- Docker daemon 没启动
- 本地还没有构建 `my-docker-app:latest`

先执行：

```bash
docker build -t my-docker-app:latest .
```

### 2. `missing plain Wasm artifact`

说明普通 Wasm 工件还没构建：

```bash
cargo build --target wasm32-wasip1 --release
```

### 3. `missing AOT Wasm artifact`

说明 AOT 工件还没生成或已经过期：

```bash
wasmedge compile target/wasm32-wasip1/release/my-app.wasm my-app-aot.wasm
```

### 4. 普通 Wasm 和 AOT 差异很大是否正常

正常。对于像 `fib` 这样的 CPU-bound workload，普通 Wasm 和 AOT 的差距主要会反映在 `internal_compute_ms` 上，而不是 `startup_overhead_ms` 上。

## 当前结论边界

当前基准可以回答：

- 普通 Wasm、AOT Wasm、Docker 在本机上的端到端延迟差异
- 这些差异中有多少来自启动，有多少来自计算
- AOT 对 CPU-bound workload 的收益是否明显

当前基准不能直接回答：

- Kubernetes Pod 级冷启动延迟
- 镜像拉取时间
- 网络请求型服务的端到端时延
- 多副本并发调度下的扩展性
