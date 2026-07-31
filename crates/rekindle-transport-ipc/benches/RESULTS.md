
usrbinkat@mithril rekindle feat/cli-tui-restructure…
❯ mise run ipc:bench
[ipc:bench] $ #!/usr/bin/env bash
[ipc:bench] rekindle-transport-ipc — Workload Benchmarks
[ipc:bench] =============================================
[ipc:bench]     Finished `bench` profile [optimized] target(s) in 0.24s
[ipc:bench]      Running benches/v3/crypto_primitives.rs (target/release/deps/v3_crypto_primitives-b3aa614e5f8d71e5)
[ipc:bench] Benchmarking emac/verify
[ipc:bench] Benchmarking emac/verify: Warming up for 3.0000 s
[ipc:bench] Benchmarking emac/verify: Collecting 30 samples in estimated 30.000 s (362M iterations)
[ipc:bench] Benchmarking emac/verify: Analyzing
[ipc:bench] emac/verify             time:   [84.403 ns 84.748 ns 85.098 ns]
[ipc:bench]                         thrpt:  [11.751 Melem/s 11.800 Melem/s 11.848 Melem/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [+1.0619% +1.6685% +2.2211%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [−2.1729% −1.6411% −1.0507%]
[ipc:bench]                         Performance has regressed.
[ipc:bench]
[ipc:bench] Benchmarking header_mac/build
[ipc:bench] Benchmarking header_mac/build: Warming up for 3.0000 s
[ipc:bench] Benchmarking header_mac/build: Collecting 30 samples in estimated 30.000 s (508M iterations)
[ipc:bench] Benchmarking header_mac/build: Analyzing
[ipc:bench] header_mac/build        time:   [57.427 ns 57.967 ns 58.621 ns]
[ipc:bench]                         thrpt:  [17.059 Melem/s 17.251 Melem/s 17.413 Melem/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−0.3141% +0.5379% +1.3919%] (p = 0.23 > 0.05)
[ipc:bench]                         thrpt:  [−1.3728% −0.5350% +0.3151%]
[ipc:bench]                         No change in performance detected.
[ipc:bench] Found 1 outliers among 30 measurements (3.33%)
[ipc:bench]   1 (3.33%) high mild
[ipc:bench] Benchmarking header_mac/verify
[ipc:bench] Benchmarking header_mac/verify: Warming up for 3.0000 s
[ipc:bench] Benchmarking header_mac/verify: Collecting 30 samples in estimated 30.000 s (360M iterations)
[ipc:bench] Benchmarking header_mac/verify: Analyzing
[ipc:bench] header_mac/verify       time:   [81.248 ns 82.155 ns 82.963 ns]
[ipc:bench]                         thrpt:  [12.054 Melem/s 12.172 Melem/s 12.308 Melem/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−0.4558% +0.3392% +1.1564%] (p = 0.42 > 0.05)
[ipc:bench]                         thrpt:  [−1.1432% −0.3380% +0.4579%]
[ipc:bench]                         No change in performance detected.
[ipc:bench]
[ipc:bench] Benchmarking aead_seal/aes256gcm/64B
[ipc:bench] Benchmarking aead_seal/aes256gcm/64B: Warming up for 3.0000 s
[ipc:bench] Benchmarking aead_seal/aes256gcm/64B: Collecting 30 samples in estimated 30.000 s (124M iterations)
[ipc:bench] Benchmarking aead_seal/aes256gcm/64B: Analyzing
[ipc:bench] aead_seal/aes256gcm/64B time:   [230.24 ns 233.59 ns 238.22 ns]
[ipc:bench]                         thrpt:  [256.22 MiB/s 261.29 MiB/s 265.09 MiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [+0.3417% +1.3159% +2.3148%] (p = 0.02 < 0.05)
[ipc:bench]                         thrpt:  [−2.2624% −1.2988% −0.3405%]
[ipc:bench]                         Change within noise threshold.
[ipc:bench] Benchmarking aead_seal/aes256gcm/1KiB
[ipc:bench] Benchmarking aead_seal/aes256gcm/1KiB: Warming up for 3.0000 s
[ipc:bench] Benchmarking aead_seal/aes256gcm/1KiB: Collecting 30 samples in estimated 30.000 s (53M iterations)
[ipc:bench] Benchmarking aead_seal/aes256gcm/1KiB: Analyzing
[ipc:bench] aead_seal/aes256gcm/1KiB
[ipc:bench]                         time:   [570.33 ns 575.13 ns 584.04 ns]
[ipc:bench]                         thrpt:  [1.6329 GiB/s 1.6582 GiB/s 1.6722 GiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−3.1197% −2.5343% −1.6186%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [+1.6452% +2.6002% +3.2202%]
[ipc:bench]                         Performance has improved.
[ipc:bench] Found 1 outliers among 30 measurements (3.33%)
[ipc:bench]   1 (3.33%) high severe
[ipc:bench] Benchmarking aead_seal/aes256gcm/64KiB
[ipc:bench] Benchmarking aead_seal/aes256gcm/64KiB: Warming up for 3.0000 s
[ipc:bench] Benchmarking aead_seal/aes256gcm/64KiB: Collecting 30 samples in estimated 30.002 s (1.7M iterations)
[ipc:bench] Benchmarking aead_seal/aes256gcm/64KiB: Analyzing
[ipc:bench] aead_seal/aes256gcm/64KiB
[ipc:bench]                         time:   [17.248 µs 17.479 µs 17.779 µs]
[ipc:bench]                         thrpt:  [3.4330 GiB/s 3.4920 GiB/s 3.5388 GiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−2.4000% −1.6833% −0.6242%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [+0.6281% +1.7122% +2.4590%]
[ipc:bench]                         Change within noise threshold.
[ipc:bench] Found 6 outliers among 30 measurements (20.00%)
[ipc:bench]   1 (3.33%) low mild
[ipc:bench]   2 (6.67%) high mild
[ipc:bench]   3 (10.00%) high severe
[ipc:bench] Benchmarking aead_seal/aes256gcm/1MiB
[ipc:bench] Benchmarking aead_seal/aes256gcm/1MiB: Warming up for 3.0000 s
[ipc:bench] Benchmarking aead_seal/aes256gcm/1MiB: Collecting 30 samples in estimated 30.069 s (104k iterations)
[ipc:bench] Benchmarking aead_seal/aes256gcm/1MiB: Analyzing
[ipc:bench] aead_seal/aes256gcm/1MiB
[ipc:bench]                         time:   [289.99 µs 290.42 µs 290.90 µs]
[ipc:bench]                         thrpt:  [3.3570 GiB/s 3.3626 GiB/s 3.3675 GiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−8.6877% −8.1830% −7.6232%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [+8.2523% +8.9123% +9.5143%]
[ipc:bench]                         Performance has improved.
[ipc:bench] Found 1 outliers among 30 measurements (3.33%)
[ipc:bench]   1 (3.33%) high severe
[ipc:bench] Benchmarking aead_seal/aes256gcm/~16MiB
[ipc:bench] Benchmarking aead_seal/aes256gcm/~16MiB: Warming up for 3.0000 s
[ipc:bench] Benchmarking aead_seal/aes256gcm/~16MiB: Collecting 30 samples in estimated 34.205 s (2325 iterations)
[ipc:bench] Benchmarking aead_seal/aes256gcm/~16MiB: Analyzing
[ipc:bench] aead_seal/aes256gcm/~16MiB
[ipc:bench]                         time:   [14.750 ms 14.838 ms 14.948 ms]
[ipc:bench]                         thrpt:  [1.0453 GiB/s 1.0530 GiB/s 1.0593 GiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−5.2150% −4.5959% −3.9475%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [+4.1097% +4.8173% +5.5020%]
[ipc:bench]                         Performance has improved.
[ipc:bench] Found 4 outliers among 30 measurements (13.33%)
[ipc:bench]   1 (3.33%) high mild
[ipc:bench]   3 (10.00%) high severe
[ipc:bench] Benchmarking aead_seal/aegis128l/64B
[ipc:bench] Benchmarking aead_seal/aegis128l/64B: Warming up for 3.0000 s
[ipc:bench] Benchmarking aead_seal/aegis128l/64B: Collecting 30 samples in estimated 30.000 s (227M iterations)
[ipc:bench] Benchmarking aead_seal/aegis128l/64B: Analyzing
[ipc:bench] aead_seal/aegis128l/64B time:   [132.60 ns 134.88 ns 137.99 ns]
[ipc:bench]                         thrpt:  [442.32 MiB/s 452.50 MiB/s 460.29 MiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−4.6211% −3.4606% −2.1622%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [+2.2100% +3.5847% +4.8450%]
[ipc:bench]                         Performance has improved.
[ipc:bench] Found 3 outliers among 30 measurements (10.00%)
[ipc:bench]   1 (3.33%) high mild
[ipc:bench]   2 (6.67%) high severe
[ipc:bench] Benchmarking aead_seal/aegis128l/1KiB
[ipc:bench] Benchmarking aead_seal/aegis128l/1KiB: Warming up for 3.0000 s
[ipc:bench] Benchmarking aead_seal/aegis128l/1KiB: Collecting 30 samples in estimated 30.000 s (92M iterations)
[ipc:bench] Benchmarking aead_seal/aegis128l/1KiB: Analyzing
[ipc:bench] aead_seal/aegis128l/1KiB
[ipc:bench]                         time:   [322.81 ns 324.80 ns 328.54 ns]
[ipc:bench]                         thrpt:  [2.9028 GiB/s 2.9362 GiB/s 2.9543 GiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−4.5447% −4.0771% −3.4264%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [+3.5479% +4.2504% +4.7610%]
[ipc:bench]                         Performance has improved.
[ipc:bench] Found 2 outliers among 30 measurements (6.67%)
[ipc:bench]   2 (6.67%) high severe
[ipc:bench] Benchmarking aead_seal/aegis128l/64KiB
[ipc:bench] Benchmarking aead_seal/aegis128l/64KiB: Warming up for 3.0000 s
[ipc:bench] Benchmarking aead_seal/aegis128l/64KiB: Collecting 30 samples in estimated 30.002 s (4.6M iterations)
[ipc:bench] Benchmarking aead_seal/aegis128l/64KiB: Analyzing
[ipc:bench] aead_seal/aegis128l/64KiB
[ipc:bench]                         time:   [6.4854 µs 6.5753 µs 6.6687 µs]
[ipc:bench]                         thrpt:  [9.1525 GiB/s 9.2825 GiB/s 9.4111 GiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−4.3472% −3.6812% −2.8588%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [+2.9429% +3.8219% +4.5448%]
[ipc:bench]                         Performance has improved.
[ipc:bench] Found 3 outliers among 30 measurements (10.00%)
[ipc:bench]   1 (3.33%) high mild
[ipc:bench]   2 (6.67%) high severe
[ipc:bench] Benchmarking aead_seal/aegis128l/1MiB
[ipc:bench] Benchmarking aead_seal/aegis128l/1MiB: Warming up for 3.0000 s
[ipc:bench] Benchmarking aead_seal/aegis128l/1MiB: Collecting 30 samples in estimated 30.003 s (269k iterations)
[ipc:bench] Benchmarking aead_seal/aegis128l/1MiB: Analyzing
[ipc:bench] aead_seal/aegis128l/1MiB
[ipc:bench]                         time:   [111.55 µs 111.86 µs 112.15 µs]
[ipc:bench]                         thrpt:  [8.7078 GiB/s 8.7302 GiB/s 8.7543 GiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−8.1840% −7.7460% −7.3165%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [+7.8941% +8.3964% +8.9134%]
[ipc:bench]                         Performance has improved.
[ipc:bench] Found 1 outliers among 30 measurements (3.33%)
[ipc:bench]   1 (3.33%) high mild
[ipc:bench] Benchmarking aead_seal/aegis128l/~16MiB
[ipc:bench] Benchmarking aead_seal/aegis128l/~16MiB: Warming up for 3.0000 s
[ipc:bench] Benchmarking aead_seal/aegis128l/~16MiB: Collecting 30 samples in estimated 31.656 s (2790 iterations)
[ipc:bench] Benchmarking aead_seal/aegis128l/~16MiB: Analyzing
[ipc:bench] aead_seal/aegis128l/~16MiB
[ipc:bench]                         time:   [11.273 ms 11.385 ms 11.527 ms]
[ipc:bench]                         thrpt:  [1.3555 GiB/s 1.3724 GiB/s 1.3861 GiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−5.0852% −4.3440% −3.5232%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [+3.6519% +4.5412% +5.3576%]
[ipc:bench]                         Performance has improved.
[ipc:bench] Found 5 outliers among 30 measurements (16.67%)
[ipc:bench]   1 (3.33%) low mild
[ipc:bench]   2 (6.67%) high mild
[ipc:bench]   2 (6.67%) high severe
[ipc:bench] Benchmarking aead_seal/aegis128x2/64B
[ipc:bench] Benchmarking aead_seal/aegis128x2/64B: Warming up for 3.0000 s
[ipc:bench] Benchmarking aead_seal/aegis128x2/64B: Collecting 30 samples in estimated 30.000 s (177M iterations)
[ipc:bench] Benchmarking aead_seal/aegis128x2/64B: Analyzing
[ipc:bench] aead_seal/aegis128x2/64B
[ipc:bench]                         time:   [168.61 ns 169.04 ns 169.44 ns]
[ipc:bench]                         thrpt:  [360.21 MiB/s 361.06 MiB/s 361.98 MiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−0.4348% −0.1162% +0.1849%] (p = 0.49 > 0.05)
[ipc:bench]                         thrpt:  [−0.1845% +0.1164% +0.4367%]
[ipc:bench]                         No change in performance detected.
[ipc:bench] Benchmarking aead_seal/aegis128x2/1KiB
[ipc:bench] Benchmarking aead_seal/aegis128x2/1KiB: Warming up for 3.0000 s
[ipc:bench] Benchmarking aead_seal/aegis128x2/1KiB: Collecting 30 samples in estimated 30.000 s (84M iterations)
[ipc:bench] Benchmarking aead_seal/aegis128x2/1KiB: Analyzing
[ipc:bench] aead_seal/aegis128x2/1KiB
[ipc:bench]                         time:   [357.60 ns 358.74 ns 359.88 ns]
[ipc:bench]                         thrpt:  [2.6500 GiB/s 2.6584 GiB/s 2.6669 GiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−5.6932% −5.1637% −4.4971%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [+4.7089% +5.4448% +6.0369%]
[ipc:bench]                         Performance has improved.
[ipc:bench] Found 1 outliers among 30 measurements (3.33%)
[ipc:bench]   1 (3.33%) high severe
[ipc:bench] Benchmarking aead_seal/aegis128x2/64KiB
[ipc:bench] Benchmarking aead_seal/aegis128x2/64KiB: Warming up for 3.0000 s
[ipc:bench] Benchmarking aead_seal/aegis128x2/64KiB: Collecting 30 samples in estimated 30.004 s (3.6M iterations)
[ipc:bench] Benchmarking aead_seal/aegis128x2/64KiB: Analyzing
[ipc:bench] aead_seal/aegis128x2/64KiB
[ipc:bench]                         time:   [8.1885 µs 8.2483 µs 8.3272 µs]
[ipc:bench]                         thrpt:  [7.3296 GiB/s 7.3997 GiB/s 7.4538 GiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [+4.7617% +6.2754% +7.8402%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [−7.2702% −5.9049% −4.5453%]
[ipc:bench]                         Performance has regressed.
[ipc:bench] Benchmarking aead_seal/aegis128x2/1MiB
[ipc:bench] Benchmarking aead_seal/aegis128x2/1MiB: Warming up for 3.0000 s
[ipc:bench] Benchmarking aead_seal/aegis128x2/1MiB: Collecting 30 samples in estimated 30.013 s (236k iterations)
[ipc:bench] Benchmarking aead_seal/aegis128x2/1MiB: Analyzing
[ipc:bench] aead_seal/aegis128x2/1MiB
[ipc:bench]                         time:   [124.51 µs 126.05 µs 127.79 µs]
[ipc:bench]                         thrpt:  [7.6420 GiB/s 7.7477 GiB/s 7.8431 GiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−6.4885% −5.7638% −4.9641%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [+5.2234% +6.1163% +6.9387%]
[ipc:bench]                         Performance has improved.
[ipc:bench] Benchmarking aead_seal/aegis128x2/~16MiB
[ipc:bench] Benchmarking aead_seal/aegis128x2/~16MiB: Warming up for 3.0000 s
[ipc:bench] Benchmarking aead_seal/aegis128x2/~16MiB: Collecting 30 samples in estimated 33.214 s (2790 iterations)
[ipc:bench] Benchmarking aead_seal/aegis128x2/~16MiB: Analyzing
[ipc:bench] aead_seal/aegis128x2/~16MiB
[ipc:bench]                         time:   [11.663 ms 11.803 ms 11.960 ms]
[ipc:bench]                         thrpt:  [1.3065 GiB/s 1.3238 GiB/s 1.3397 GiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−4.5839% −3.5442% −2.4918%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [+2.5555% +3.6744% +4.8041%]
[ipc:bench]                         Performance has improved.
[ipc:bench]
[ipc:bench] Benchmarking aead_open/aes256gcm/64B
[ipc:bench] Benchmarking aead_open/aes256gcm/64B: Warming up for 3.0000 s
[ipc:bench] Benchmarking aead_open/aes256gcm/64B: Collecting 30 samples in estimated 30.000 s (141M iterations)
[ipc:bench] Benchmarking aead_open/aes256gcm/64B: Analyzing
[ipc:bench] aead_open/aes256gcm/64B time:   [205.08 ns 205.98 ns 206.91 ns]
[ipc:bench]                         thrpt:  [294.99 MiB/s 296.32 MiB/s 297.61 MiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−3.4489% −2.7976% −2.1176%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [+2.1635% +2.8781% +3.5721%]
[ipc:bench]                         Performance has improved.
[ipc:bench] Found 4 outliers among 30 measurements (13.33%)
[ipc:bench]   3 (10.00%) high mild
[ipc:bench]   1 (3.33%) high severe
[ipc:bench] Benchmarking aead_open/aes256gcm/1KiB
[ipc:bench] Benchmarking aead_open/aes256gcm/1KiB: Warming up for 3.0000 s
[ipc:bench] Benchmarking aead_open/aes256gcm/1KiB: Collecting 30 samples in estimated 30.000 s (60M iterations)
[ipc:bench] Benchmarking aead_open/aes256gcm/1KiB: Analyzing
[ipc:bench] aead_open/aes256gcm/1KiB
[ipc:bench]                         time:   [495.47 ns 496.36 ns 497.28 ns]
[ipc:bench]                         thrpt:  [1.9178 GiB/s 1.9213 GiB/s 1.9248 GiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [+0.9796% +1.2445% +1.5077%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [−1.4853% −1.2292% −0.9701%]
[ipc:bench]                         Change within noise threshold.
[ipc:bench] Found 1 outliers among 30 measurements (3.33%)
[ipc:bench]   1 (3.33%) high mild
[ipc:bench] Benchmarking aead_open/aes256gcm/64KiB
[ipc:bench] Benchmarking aead_open/aes256gcm/64KiB: Warming up for 3.0000 s
[ipc:bench] Benchmarking aead_open/aes256gcm/64KiB: Collecting 30 samples in estimated 30.003 s (2.1M iterations)
[ipc:bench] Benchmarking aead_open/aes256gcm/64KiB: Analyzing
[ipc:bench] aead_open/aes256gcm/64KiB
[ipc:bench]                         time:   [14.245 µs 14.270 µs 14.291 µs]
[ipc:bench]                         thrpt:  [4.2710 GiB/s 4.2771 GiB/s 4.2846 GiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−6.2890% −5.8803% −5.4753%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [+5.7924% +6.2477% +6.7111%]
[ipc:bench]                         Performance has improved.
[ipc:bench] Found 4 outliers among 30 measurements (13.33%)
[ipc:bench]   2 (6.67%) low mild
[ipc:bench]   1 (3.33%) high mild
[ipc:bench]   1 (3.33%) high severe
[ipc:bench] Benchmarking aead_open/aes256gcm/1MiB
[ipc:bench] Benchmarking aead_open/aes256gcm/1MiB: Warming up for 3.0000 s
[ipc:bench] Benchmarking aead_open/aes256gcm/1MiB: Collecting 30 samples in estimated 30.084 s (125k iterations)
[ipc:bench] Benchmarking aead_open/aes256gcm/1MiB: Analyzing
[ipc:bench] aead_open/aes256gcm/1MiB
[ipc:bench]                         time:   [232.51 µs 234.22 µs 236.24 µs]
[ipc:bench]                         thrpt:  [4.1337 GiB/s 4.1693 GiB/s 4.2001 GiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−1.7825% −1.2649% −0.7883%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [+0.7945% +1.2811% +1.8148%]
[ipc:bench]                         Change within noise threshold.
[ipc:bench] Found 6 outliers among 30 measurements (20.00%)
[ipc:bench]   4 (13.33%) low mild
[ipc:bench]   2 (6.67%) high mild
[ipc:bench] Benchmarking aead_open/aes256gcm/~16MiB
[ipc:bench] Benchmarking aead_open/aes256gcm/~16MiB: Warming up for 3.0000 s
[ipc:bench] Benchmarking aead_open/aes256gcm/~16MiB: Collecting 30 samples in estimated 31.317 s (7905 iterations)
[ipc:bench] Benchmarking aead_open/aes256gcm/~16MiB: Analyzing
[ipc:bench] aead_open/aes256gcm/~16MiB
[ipc:bench]                         time:   [4.0298 ms 4.0636 ms 4.0979 ms]
[ipc:bench]                         thrpt:  [3.8129 GiB/s 3.8451 GiB/s 3.8774 GiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−0.3615% +0.3086% +0.9920%] (p = 0.37 > 0.05)
[ipc:bench]                         thrpt:  [−0.9823% −0.3077% +0.3628%]
[ipc:bench]                         No change in performance detected.
[ipc:bench] Found 2 outliers among 30 measurements (6.67%)
[ipc:bench]   2 (6.67%) high mild
[ipc:bench] Benchmarking aead_open/aegis128l/64B
[ipc:bench] Benchmarking aead_open/aegis128l/64B: Warming up for 3.0000 s
[ipc:bench] Benchmarking aead_open/aegis128l/64B: Collecting 30 samples in estimated 30.000 s (216M iterations)
[ipc:bench] Benchmarking aead_open/aegis128l/64B: Analyzing
[ipc:bench] aead_open/aegis128l/64B time:   [141.37 ns 142.48 ns 143.98 ns]
[ipc:bench]                         thrpt:  [423.92 MiB/s 428.37 MiB/s 431.75 MiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−1.3458% −0.2588% +0.8615%] (p = 0.68 > 0.05)
[ipc:bench]                         thrpt:  [−0.8541% +0.2594% +1.3641%]
[ipc:bench]                         No change in performance detected.
[ipc:bench] Found 1 outliers among 30 measurements (3.33%)
[ipc:bench]   1 (3.33%) high mild
[ipc:bench] Benchmarking aead_open/aegis128l/1KiB
[ipc:bench] Benchmarking aead_open/aegis128l/1KiB: Warming up for 3.0000 s
[ipc:bench] Benchmarking aead_open/aegis128l/1KiB: Collecting 30 samples in estimated 30.000 s (101M iterations)
[ipc:bench] Benchmarking aead_open/aegis128l/1KiB: Analyzing
[ipc:bench] aead_open/aegis128l/1KiB
[ipc:bench]                         time:   [290.09 ns 292.80 ns 296.14 ns]
[ipc:bench]                         thrpt:  [3.2203 GiB/s 3.2571 GiB/s 3.2876 GiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−2.2640% −1.5164% −0.7219%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [+0.7271% +1.5398% +2.3164%]
[ipc:bench]                         Change within noise threshold.
[ipc:bench] Found 2 outliers among 30 measurements (6.67%)
[ipc:bench]   2 (6.67%) high mild
[ipc:bench] Benchmarking aead_open/aegis128l/64KiB
[ipc:bench] Benchmarking aead_open/aegis128l/64KiB: Warming up for 3.0000 s
[ipc:bench] Benchmarking aead_open/aegis128l/64KiB: Collecting 30 samples in estimated 30.000 s (6.1M iterations)
[ipc:bench] Benchmarking aead_open/aegis128l/64KiB: Analyzing
[ipc:bench] aead_open/aegis128l/64KiB
[ipc:bench]                         time:   [4.9256 µs 4.9509 µs 4.9820 µs]
[ipc:bench]                         thrpt:  [12.251 GiB/s 12.328 GiB/s 12.392 GiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−5.9380% −5.4536% −4.9696%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [+5.2295% +5.7682% +6.3129%]
[ipc:bench]                         Performance has improved.
[ipc:bench] Benchmarking aead_open/aegis128l/1MiB
[ipc:bench] Benchmarking aead_open/aegis128l/1MiB: Warming up for 3.0000 s
[ipc:bench] Benchmarking aead_open/aegis128l/1MiB: Collecting 30 samples in estimated 30.027 s (354k iterations)
[ipc:bench] Benchmarking aead_open/aegis128l/1MiB: Analyzing
[ipc:bench] aead_open/aegis128l/1MiB
[ipc:bench]                         time:   [83.532 µs 84.220 µs 84.780 µs]
[ipc:bench]                         thrpt:  [11.519 GiB/s 11.595 GiB/s 11.691 GiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−4.9087% −4.1393% −3.3464%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [+3.4622% +4.3180% +5.1621%]
[ipc:bench]                         Performance has improved.
[ipc:bench] Benchmarking aead_open/aegis128l/~16MiB
[ipc:bench] Benchmarking aead_open/aegis128l/~16MiB: Warming up for 3.0000 s
[ipc:bench] Benchmarking aead_open/aegis128l/~16MiB: Collecting 30 samples in estimated 30.270 s (13k iterations)
[ipc:bench] Benchmarking aead_open/aegis128l/~16MiB: Analyzing
[ipc:bench] aead_open/aegis128l/~16MiB
[ipc:bench]                         time:   [2.1498 ms 2.1997 ms 2.2565 ms]
[ipc:bench]                         thrpt:  [6.9244 GiB/s 7.1031 GiB/s 7.2679 GiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−3.5572% −1.5423% +0.6994%] (p = 0.17 > 0.05)
[ipc:bench]                         thrpt:  [−0.6945% +1.5664% +3.6884%]
[ipc:bench]                         No change in performance detected.
[ipc:bench] Benchmarking aead_open/aegis128x2/64B
[ipc:bench] Benchmarking aead_open/aegis128x2/64B: Warming up for 3.0000 s
[ipc:bench] Benchmarking aead_open/aegis128x2/64B: Collecting 30 samples in estimated 30.000 s (171M iterations)
[ipc:bench] Benchmarking aead_open/aegis128x2/64B: Analyzing
[ipc:bench] aead_open/aegis128x2/64B
[ipc:bench]                         time:   [175.71 ns 176.92 ns 178.81 ns]
[ipc:bench]                         thrpt:  [341.33 MiB/s 344.98 MiB/s 347.37 MiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−0.2841% +0.6605% +1.8204%] (p = 0.27 > 0.05)
[ipc:bench]                         thrpt:  [−1.7879% −0.6562% +0.2849%]
[ipc:bench]                         No change in performance detected.
[ipc:bench] Found 3 outliers among 30 measurements (10.00%)
[ipc:bench]   1 (3.33%) high mild
[ipc:bench]   2 (6.67%) high severe
[ipc:bench] Benchmarking aead_open/aegis128x2/1KiB
[ipc:bench] Benchmarking aead_open/aegis128x2/1KiB: Warming up for 3.0000 s
[ipc:bench] Benchmarking aead_open/aegis128x2/1KiB: Collecting 30 samples in estimated 30.000 s (91M iterations)
[ipc:bench] Benchmarking aead_open/aegis128x2/1KiB: Analyzing
[ipc:bench] aead_open/aegis128x2/1KiB
[ipc:bench]                         time:   [327.15 ns 328.08 ns 329.07 ns]
[ipc:bench]                         thrpt:  [2.8981 GiB/s 2.9068 GiB/s 2.9151 GiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [+0.5774% +0.9649% +1.3750%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [−1.3564% −0.9556% −0.5741%]
[ipc:bench]                         Change within noise threshold.
[ipc:bench] Found 1 outliers among 30 measurements (3.33%)
[ipc:bench]   1 (3.33%) high mild
[ipc:bench] Benchmarking aead_open/aegis128x2/64KiB
[ipc:bench] Benchmarking aead_open/aegis128x2/64KiB: Warming up for 3.0000 s
[ipc:bench] Benchmarking aead_open/aegis128x2/64KiB: Collecting 30 samples in estimated 30.001 s (5.2M iterations)
[ipc:bench] Benchmarking aead_open/aegis128x2/64KiB: Analyzing
[ipc:bench] aead_open/aegis128x2/64KiB
[ipc:bench]                         time:   [5.5427 µs 5.5642 µs 5.5891 µs]
[ipc:bench]                         thrpt:  [10.920 GiB/s 10.969 GiB/s 11.012 GiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [+1.1919% +2.1594% +3.2818%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [−3.1775% −2.1138% −1.1779%]
[ipc:bench]                         Performance has regressed.
[ipc:bench] Found 3 outliers among 30 measurements (10.00%)
[ipc:bench]   1 (3.33%) high mild
[ipc:bench]   2 (6.67%) high severe
[ipc:bench] Benchmarking aead_open/aegis128x2/1MiB
[ipc:bench] Benchmarking aead_open/aegis128x2/1MiB: Warming up for 3.0000 s
[ipc:bench] Benchmarking aead_open/aegis128x2/1MiB: Collecting 30 samples in estimated 30.002 s (326k iterations)
[ipc:bench] Benchmarking aead_open/aegis128x2/1MiB: Analyzing
[ipc:bench] aead_open/aegis128x2/1MiB
[ipc:bench]                         time:   [91.746 µs 92.339 µs 93.012 µs]
[ipc:bench]                         thrpt:  [10.499 GiB/s 10.576 GiB/s 10.644 GiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−9.6393% −9.1411% −8.6107%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [+9.4220% +10.061% +10.668%]
[ipc:bench]                         Performance has improved.
[ipc:bench] Found 3 outliers among 30 measurements (10.00%)
[ipc:bench]   2 (6.67%) high mild
[ipc:bench]   1 (3.33%) high severe
[ipc:bench] Benchmarking aead_open/aegis128x2/~16MiB
[ipc:bench] Benchmarking aead_open/aegis128x2/~16MiB: Warming up for 3.0000 s
[ipc:bench] Benchmarking aead_open/aegis128x2/~16MiB: Collecting 30 samples in estimated 30.885 s (13k iterations)
[ipc:bench] Benchmarking aead_open/aegis128x2/~16MiB: Analyzing
[ipc:bench] aead_open/aegis128x2/~16MiB
[ipc:bench]                         time:   [2.3397 ms 2.3859 ms 2.4342 ms]
[ipc:bench]                         thrpt:  [6.4189 GiB/s 6.5489 GiB/s 6.6783 GiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−3.1668% −1.5307% +0.0748%] (p = 0.08 > 0.05)
[ipc:bench]                         thrpt:  [−0.0748% +1.5545% +3.2704%]
[ipc:bench]                         No change in performance detected.
[ipc:bench]
[ipc:bench] Benchmarking aead_seal_into/aes256gcm/64B
[ipc:bench] Benchmarking aead_seal_into/aes256gcm/64B: Warming up for 3.0000 s
[ipc:bench] Benchmarking aead_seal_into/aes256gcm/64B: Collecting 30 samples in estimated 30.000 s (159M iterations)
[ipc:bench] Benchmarking aead_seal_into/aes256gcm/64B: Analyzing
[ipc:bench] aead_seal_into/aes256gcm/64B
[ipc:bench]                         time:   [184.87 ns 185.76 ns 186.68 ns]
[ipc:bench]                         thrpt:  [326.96 MiB/s 328.57 MiB/s 330.15 MiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [+0.1248% +0.8205% +1.4749%] (p = 0.02 < 0.05)
[ipc:bench]                         thrpt:  [−1.4534% −0.8138% −0.1246%]
[ipc:bench]                         Change within noise threshold.
[ipc:bench] Benchmarking aead_seal_into/aes256gcm/1KiB
[ipc:bench] Benchmarking aead_seal_into/aes256gcm/1KiB: Warming up for 3.0000 s
[ipc:bench] Benchmarking aead_seal_into/aes256gcm/1KiB: Collecting 30 samples in estimated 30.000 s (76M iterations)
[ipc:bench] Benchmarking aead_seal_into/aes256gcm/1KiB: Analyzing
[ipc:bench] aead_seal_into/aes256gcm/1KiB
[ipc:bench]                         time:   [395.10 ns 397.66 ns 399.89 ns]
[ipc:bench]                         thrpt:  [2.3848 GiB/s 2.3982 GiB/s 2.4138 GiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−4.0998% −3.4788% −2.8615%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [+2.9458% +3.6042% +4.2751%]
[ipc:bench]                         Performance has improved.
[ipc:bench] Found 1 outliers among 30 measurements (3.33%)
[ipc:bench]   1 (3.33%) high mild
[ipc:bench] Benchmarking aead_seal_into/aes256gcm/64KiB
[ipc:bench] Benchmarking aead_seal_into/aes256gcm/64KiB: Warming up for 3.0000 s
[ipc:bench] Benchmarking aead_seal_into/aes256gcm/64KiB: Collecting 30 samples in estimated 30.007 s (1.9M iterations)
[ipc:bench] Benchmarking aead_seal_into/aes256gcm/64KiB: Analyzing
[ipc:bench] aead_seal_into/aes256gcm/64KiB
[ipc:bench]                         time:   [15.289 µs 15.389 µs 15.549 µs]
[ipc:bench]                         thrpt:  [3.9253 GiB/s 3.9662 GiB/s 3.9920 GiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [+0.0314% +0.5320% +1.0594%] (p = 0.06 > 0.05)
[ipc:bench]                         thrpt:  [−1.0483% −0.5292% −0.0314%]
[ipc:bench]                         No change in performance detected.
[ipc:bench] Found 2 outliers among 30 measurements (6.67%)
[ipc:bench]   1 (3.33%) high mild
[ipc:bench]   1 (3.33%) high severe
[ipc:bench] Benchmarking aead_seal_into/aes256gcm/1MiB
[ipc:bench] Benchmarking aead_seal_into/aes256gcm/1MiB: Warming up for 3.0000 s
[ipc:bench] Benchmarking aead_seal_into/aes256gcm/1MiB: Collecting 30 samples in estimated 30.068 s (121k iterations)
[ipc:bench] Benchmarking aead_seal_into/aes256gcm/1MiB: Analyzing
[ipc:bench] aead_seal_into/aes256gcm/1MiB
[ipc:bench]                         time:   [244.12 µs 244.95 µs 246.13 µs]
[ipc:bench]                         thrpt:  [3.9677 GiB/s 3.9868 GiB/s 4.0004 GiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−4.4306% −3.8025% −3.1339%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [+3.2353% +3.9528% +4.6360%]
[ipc:bench]                         Performance has improved.
[ipc:bench] Found 1 outliers among 30 measurements (3.33%)
[ipc:bench]   1 (3.33%) high mild
[ipc:bench] Benchmarking aead_seal_into/aes256gcm/~16MiB
[ipc:bench] Benchmarking aead_seal_into/aes256gcm/~16MiB: Warming up for 3.0000 s
[ipc:bench] Benchmarking aead_seal_into/aes256gcm/~16MiB: Collecting 30 samples in estimated 31.237 s (7440 iterations)
[ipc:bench] Benchmarking aead_seal_into/aes256gcm/~16MiB: Analyzing
[ipc:bench] aead_seal_into/aes256gcm/~16MiB
[ipc:bench]                         time:   [4.1442 ms 4.1956 ms 4.2826 ms]
[ipc:bench]                         thrpt:  [3.6485 GiB/s 3.7241 GiB/s 3.7704 GiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−10.179% −9.2905% −8.0775%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [+8.7873% +10.242% +11.333%]
[ipc:bench]                         Performance has improved.
[ipc:bench] Found 2 outliers among 30 measurements (6.67%)
[ipc:bench]   2 (6.67%) high severe
[ipc:bench] Benchmarking aead_seal_into/aegis128l/64B
[ipc:bench] Benchmarking aead_seal_into/aegis128l/64B: Warming up for 3.0000 s
[ipc:bench] Benchmarking aead_seal_into/aegis128l/64B: Collecting 30 samples in estimated 30.000 s (317M iterations)
[ipc:bench] Benchmarking aead_seal_into/aegis128l/64B: Analyzing
[ipc:bench] aead_seal_into/aegis128l/64B
[ipc:bench]                         time:   [93.705 ns 94.304 ns 95.081 ns]
[ipc:bench]                         thrpt:  [641.93 MiB/s 647.22 MiB/s 651.36 MiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−3.7834% −3.2781% −2.6700%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [+2.7433% +3.3892% +3.9322%]
[ipc:bench]                         Performance has improved.
[ipc:bench] Found 3 outliers among 30 measurements (10.00%)
[ipc:bench]   3 (10.00%) high mild
[ipc:bench] Benchmarking aead_seal_into/aegis128l/1KiB
[ipc:bench] Benchmarking aead_seal_into/aegis128l/1KiB: Warming up for 3.0000 s
[ipc:bench] Benchmarking aead_seal_into/aegis128l/1KiB: Collecting 30 samples in estimated 30.000 s (197M iterations)
[ipc:bench] Benchmarking aead_seal_into/aegis128l/1KiB: Analyzing
[ipc:bench] aead_seal_into/aegis128l/1KiB
[ipc:bench]                         time:   [147.01 ns 147.83 ns 148.92 ns]
[ipc:bench]                         thrpt:  [6.4041 GiB/s 6.4510 GiB/s 6.4870 GiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−4.0861% −3.4015% −2.7904%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [+2.8705% +3.5212% +4.2602%]
[ipc:bench]                         Performance has improved.
[ipc:bench] Benchmarking aead_seal_into/aegis128l/64KiB
[ipc:bench] Benchmarking aead_seal_into/aegis128l/64KiB: Warming up for 3.0000 s
[ipc:bench] Benchmarking aead_seal_into/aegis128l/64KiB: Collecting 30 samples in estimated 30.001 s (7.9M iterations)
[ipc:bench] Benchmarking aead_seal_into/aegis128l/64KiB: Analyzing
[ipc:bench] aead_seal_into/aegis128l/64KiB
[ipc:bench]                         time:   [3.8245 µs 3.8492 µs 3.8680 µs]
[ipc:bench]                         thrpt:  [15.779 GiB/s 15.857 GiB/s 15.959 GiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−5.1272% −4.4663% −3.8200%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [+3.9717% +4.6751% +5.4043%]
[ipc:bench]                         Performance has improved.
[ipc:bench] Benchmarking aead_seal_into/aegis128l/1MiB
[ipc:bench] Benchmarking aead_seal_into/aegis128l/1MiB: Warming up for 3.0000 s
[ipc:bench] Benchmarking aead_seal_into/aegis128l/1MiB: Collecting 30 samples in estimated 30.019 s (471k iterations)
[ipc:bench] Benchmarking aead_seal_into/aegis128l/1MiB: Analyzing
[ipc:bench] aead_seal_into/aegis128l/1MiB
[ipc:bench]                         time:   [63.688 µs 63.768 µs 63.860 µs]
[ipc:bench]                         thrpt:  [15.292 GiB/s 15.314 GiB/s 15.333 GiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−4.8145% −4.4515% −4.1015%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [+4.2770% +4.6589% +5.0580%]
[ipc:bench]                         Performance has improved.
[ipc:bench] Found 2 outliers among 30 measurements (6.67%)
[ipc:bench]   2 (6.67%) high mild
[ipc:bench] Benchmarking aead_seal_into/aegis128l/~16MiB
[ipc:bench] Benchmarking aead_seal_into/aegis128l/~16MiB: Warming up for 3.0000 s
[ipc:bench] Benchmarking aead_seal_into/aegis128l/~16MiB: Collecting 30 samples in estimated 30.693 s (20k iterations)
[ipc:bench] Benchmarking aead_seal_into/aegis128l/~16MiB: Analyzing
[ipc:bench] aead_seal_into/aegis128l/~16MiB
[ipc:bench]                         time:   [1.5378 ms 1.5467 ms 1.5555 ms]
[ipc:bench]                         thrpt:  [10.045 GiB/s 10.102 GiB/s 10.160 GiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−13.785% −12.525% −10.858%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [+12.181% +14.318% +15.989%]
[ipc:bench]                         Performance has improved.
[ipc:bench] Found 2 outliers among 30 measurements (6.67%)
[ipc:bench]   2 (6.67%) high severe
[ipc:bench] Benchmarking aead_seal_into/aegis128x2/64B
[ipc:bench] Benchmarking aead_seal_into/aegis128x2/64B: Warming up for 3.0000 s
[ipc:bench] Benchmarking aead_seal_into/aegis128x2/64B: Collecting 30 samples in estimated 30.000 s (240M iterations)
[ipc:bench] Benchmarking aead_seal_into/aegis128x2/64B: Analyzing
[ipc:bench] aead_seal_into/aegis128x2/64B
[ipc:bench]                         time:   [123.89 ns 124.75 ns 125.46 ns]
[ipc:bench]                         thrpt:  [486.50 MiB/s 489.24 MiB/s 492.64 MiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−3.0926% −2.5025% −1.9361%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [+1.9743% +2.5667% +3.1913%]
[ipc:bench]                         Performance has improved.
[ipc:bench] Found 2 outliers among 30 measurements (6.67%)
[ipc:bench]   1 (3.33%) high mild
[ipc:bench]   1 (3.33%) high severe
[ipc:bench] Benchmarking aead_seal_into/aegis128x2/1KiB
[ipc:bench] Benchmarking aead_seal_into/aegis128x2/1KiB: Warming up for 3.0000 s
[ipc:bench] Benchmarking aead_seal_into/aegis128x2/1KiB: Collecting 30 samples in estimated 30.000 s (158M iterations)
[ipc:bench] Benchmarking aead_seal_into/aegis128x2/1KiB: Analyzing
[ipc:bench] aead_seal_into/aegis128x2/1KiB
[ipc:bench]                         time:   [188.41 ns 188.81 ns 189.33 ns]
[ipc:bench]                         thrpt:  [5.0370 GiB/s 5.0509 GiB/s 5.0616 GiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [+1.7170% +3.0012% +5.1379%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [−4.8868% −2.9137% −1.6880%]
[ipc:bench]                         Performance has regressed.
[ipc:bench] Found 3 outliers among 30 measurements (10.00%)
[ipc:bench]   3 (10.00%) high severe
[ipc:bench] Benchmarking aead_seal_into/aegis128x2/64KiB
[ipc:bench] Benchmarking aead_seal_into/aegis128x2/64KiB: Warming up for 3.0000 s
[ipc:bench] Benchmarking aead_seal_into/aegis128x2/64KiB: Collecting 30 samples in estimated 30.002 s (5.9M iterations)
[ipc:bench] Benchmarking aead_seal_into/aegis128x2/64KiB: Analyzing
[ipc:bench] aead_seal_into/aegis128x2/64KiB
[ipc:bench]                         time:   [4.9419 µs 4.9735 µs 5.0102 µs]
[ipc:bench]                         thrpt:  [12.182 GiB/s 12.272 GiB/s 12.350 GiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [+5.6934% +6.1889% +6.7558%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [−6.3283% −5.8282% −5.3867%]
[ipc:bench]                         Performance has regressed.
[ipc:bench] Found 2 outliers among 30 measurements (6.67%)
[ipc:bench]   1 (3.33%) low mild
[ipc:bench]   1 (3.33%) high mild
[ipc:bench] Benchmarking aead_seal_into/aegis128x2/1MiB
[ipc:bench] Benchmarking aead_seal_into/aegis128x2/1MiB: Warming up for 3.0000 s
[ipc:bench] Benchmarking aead_seal_into/aegis128x2/1MiB: Collecting 30 samples in estimated 30.001 s (359k iterations)
[ipc:bench] Benchmarking aead_seal_into/aegis128x2/1MiB: Analyzing
[ipc:bench] aead_seal_into/aegis128x2/1MiB
[ipc:bench]                         time:   [79.787 µs 80.555 µs 81.630 µs]
[ipc:bench]                         thrpt:  [11.963 GiB/s 12.123 GiB/s 12.240 GiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−2.3464% −0.6149% +1.1688%] (p = 0.52 > 0.05)
[ipc:bench]                         thrpt:  [−1.1553% +0.6187% +2.4027%]
[ipc:bench]                         No change in performance detected.
[ipc:bench] Found 1 outliers among 30 measurements (3.33%)
[ipc:bench]   1 (3.33%) high mild
[ipc:bench] Benchmarking aead_seal_into/aegis128x2/~16MiB
[ipc:bench] Benchmarking aead_seal_into/aegis128x2/~16MiB: Warming up for 3.0000 s
[ipc:bench] Benchmarking aead_seal_into/aegis128x2/~16MiB: Collecting 30 samples in estimated 30.349 s (14k iterations)
[ipc:bench] Benchmarking aead_seal_into/aegis128x2/~16MiB: Analyzing
[ipc:bench] aead_seal_into/aegis128x2/~16MiB
[ipc:bench]                         time:   [1.7889 ms 1.8397 ms 1.9004 ms]
[ipc:bench]                         thrpt:  [8.2220 GiB/s 8.4930 GiB/s 8.7342 GiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−5.8159% −3.9259% −1.6296%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [+1.6566% +4.0863% +6.1750%]
[ipc:bench]                         Performance has improved.
[ipc:bench] Found 3 outliers among 30 measurements (10.00%)
[ipc:bench]   3 (10.00%) high mild
[ipc:bench]
[ipc:bench] Benchmarking audit_chain/link_advance
[ipc:bench] Benchmarking audit_chain/link_advance: Warming up for 3.0000 s
[ipc:bench] Benchmarking audit_chain/link_advance: Collecting 30 samples in estimated 30.000 s (166M iterations)
[ipc:bench] Benchmarking audit_chain/link_advance: Analyzing
[ipc:bench] audit_chain/link_advance
[ipc:bench]                         time:   [183.08 ns 184.35 ns 186.03 ns]
[ipc:bench]                         thrpt:  [5.3754 Melem/s 5.4243 Melem/s 5.4622 Melem/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [+4.3831% +5.1916% +5.9925%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [−5.6537% −4.9354% −4.1991%]
[ipc:bench]                         Performance has regressed.
[ipc:bench]
[ipc:bench] Benchmarking audit_chain_sustained/10k_links
[ipc:bench] Benchmarking audit_chain_sustained/10k_links: Warming up for 3.0000 s
[ipc:bench] Benchmarking audit_chain_sustained/10k_links: Collecting 30 samples in estimated 30.205 s (18k iterations)
[ipc:bench] Benchmarking audit_chain_sustained/10k_links: Analyzing
[ipc:bench] audit_chain_sustained/10k_links
[ipc:bench]                         time:   [1.6329 ms 1.6434 ms 1.6542 ms]
[ipc:bench]                         thrpt:  [6.0453 Melem/s 6.0850 Melem/s 6.1241 Melem/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−2.9999% −2.3109% −1.6761%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [+1.7047% +2.3656% +3.0927%]
[ipc:bench]                         Performance has improved.
[ipc:bench] Found 2 outliers among 30 measurements (6.67%)
[ipc:bench]   2 (6.67%) high mild
[ipc:bench]
[ipc:bench] Benchmarking emac_density/200k_burst
[ipc:bench] Benchmarking emac_density/200k_burst: Warming up for 3.0000 s
[ipc:bench] Benchmarking emac_density/200k_burst: Collecting 10 samples in estimated 60.030 s (3740 iterations)
[ipc:bench] Benchmarking emac_density/200k_burst: Analyzing
[ipc:bench] emac_density/200k_burst time:   [15.958 ms 16.053 ms 16.167 ms]
[ipc:bench]                         thrpt:  [12.371 Melem/s 12.459 Melem/s 12.533 Melem/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−2.4564% −1.9358% −1.3614%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [+1.3802% +1.9740% +2.5183%]
[ipc:bench]                         Performance has improved.
[ipc:bench] Found 1 outliers among 10 measurements (10.00%)
[ipc:bench]   1 (10.00%) high mild
[ipc:bench] Benchmarking reassembler/in_order_1KiB
[ipc:bench] Benchmarking reassembler/in_order_1KiB: Warming up for 3.0000 s
[ipc:bench]
[ipc:bench] Benchmarking reassembler/in_order_1KiB: Collecting 30 samples in estimated 30.002 s (3.3M iterations)
[ipc:bench] Benchmarking reassembler/in_order_1KiB: Analyzing
[ipc:bench] reassembler/in_order_1KiB
[ipc:bench]                         time:   [9.0311 µs 9.0604 µs 9.0879 µs]
[ipc:bench]                         thrpt:  [107.46 MiB/s 107.78 MiB/s 108.13 MiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−2.5311% −2.1092% −1.7107%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [+1.7405% +2.1546% +2.5968%]
[ipc:bench]                         Performance has improved.
[ipc:bench]
[ipc:bench] Benchmarking reassembler_ooo/50pct_ooo_1KiB
[ipc:bench] Benchmarking reassembler_ooo/50pct_ooo_1KiB: Warming up for 3.0000 s
[ipc:bench] Benchmarking reassembler_ooo/50pct_ooo_1KiB: Collecting 30 samples in estimated 30.004 s (2.4M iterations)
[ipc:bench] Benchmarking reassembler_ooo/50pct_ooo_1KiB: Analyzing
[ipc:bench] reassembler_ooo/50pct_ooo_1KiB
[ipc:bench]                         time:   [12.852 µs 12.917 µs 12.970 µs]
[ipc:bench]                         thrpt:  [75.296 MiB/s 75.600 MiB/s 75.984 MiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−1.9791% −0.9744% −0.0135%] (p = 0.06 > 0.05)
[ipc:bench]                         thrpt:  [+0.0135% +0.9840% +2.0191%]
[ipc:bench]                         No change in performance detected.
[ipc:bench]
[ipc:bench] Benchmarking reassembler_16mib/4_chunks_in_order
[ipc:bench] Benchmarking reassembler_16mib/4_chunks_in_order: Warming up for 3.0000 s
[ipc:bench] Benchmarking reassembler_16mib/4_chunks_in_order: Collecting 10 samples in estimated 60.070 s (21k iterations)
[ipc:bench] Benchmarking reassembler_16mib/4_chunks_in_order: Analyzing
[ipc:bench] reassembler_16mib/4_chunks_in_order
[ipc:bench]                         time:   [2.8985 ms 2.9239 ms 2.9414 ms]
[ipc:bench]                         thrpt:  [21.248 GiB/s 21.375 GiB/s 21.563 GiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−10.059% −8.8124% −7.7581%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [+8.4106% +9.6640% +11.184%]
[ipc:bench]                         Performance has improved.
[ipc:bench] Found 2 outliers among 10 measurements (20.00%)
[ipc:bench]   2 (20.00%) low mild
[ipc:bench]
[ipc:bench] Benchmarking pool/acquire_release
[ipc:bench] Benchmarking pool/acquire_release: Warming up for 3.0000 s
[ipc:bench] Benchmarking pool/acquire_release: Collecting 30 samples in estimated 30.000 s (962M iterations)
[ipc:bench] Benchmarking pool/acquire_release: Analyzing
[ipc:bench] pool/acquire_release    time:   [30.974 ns 31.097 ns 31.238 ns]
[ipc:bench]                         thrpt:  [32.012 Melem/s 32.158 Melem/s 32.285 Melem/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−3.0776% −2.5867% −2.1014%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [+2.1465% +2.6554% +3.1753%]
[ipc:bench]                         Performance has improved.
[ipc:bench] Found 1 outliers among 30 measurements (3.33%)
[ipc:bench]   1 (3.33%) low mild
[ipc:bench]
[ipc:bench] Benchmarking pool_contention/2_threads
[ipc:bench] Benchmarking pool_contention/2_threads: Warming up for 3.0000 s
[ipc:bench] Benchmarking pool_contention/2_threads: Collecting 30 samples in estimated 30.000 s (480M iterations)
[ipc:bench] Benchmarking pool_contention/2_threads: Analyzing
[ipc:bench] pool_contention/2_threads
[ipc:bench]                         time:   [66.921 ns 71.677 ns 75.653 ns]
[ipc:bench]                         thrpt:  [13.218 Melem/s 13.951 Melem/s 14.943 Melem/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [+19.227% +25.581% +31.918%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [−24.196% −20.370% −16.127%]
[ipc:bench]                         Performance has regressed.
[ipc:bench] Found 5 outliers among 30 measurements (16.67%)
[ipc:bench]   5 (16.67%) low mild
[ipc:bench] Benchmarking pool_contention/4_threads
[ipc:bench] Benchmarking pool_contention/4_threads: Warming up for 3.0000 s
[ipc:bench] Benchmarking pool_contention/4_threads: Collecting 30 samples in estimated 30.000 s (223M iterations)
[ipc:bench] Benchmarking pool_contention/4_threads: Analyzing
[ipc:bench] pool_contention/4_threads
[ipc:bench]                         time:   [133.81 ns 135.33 ns 136.94 ns]
[ipc:bench]                         thrpt:  [7.3025 Melem/s 7.3894 Melem/s 7.4735 Melem/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−3.3692% −2.0175% −0.5687%] (p = 0.01 < 0.05)
[ipc:bench]                         thrpt:  [+0.5719% +2.0591% +3.4867%]
[ipc:bench]                         Change within noise threshold.
[ipc:bench] Benchmarking pool_contention/8_threads
[ipc:bench] Benchmarking pool_contention/8_threads: Warming up for 3.0000 s
[ipc:bench] Benchmarking pool_contention/8_threads: Collecting 30 samples in estimated 30.000 s (176M iterations)
[ipc:bench] Benchmarking pool_contention/8_threads: Analyzing
[ipc:bench] pool_contention/8_threads
[ipc:bench]                         time:   [169.39 ns 169.82 ns 170.16 ns]
[ipc:bench]                         thrpt:  [5.8769 Melem/s 5.8885 Melem/s 5.9035 Melem/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−3.6027% −2.9052% −2.2447%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [+2.2963% +2.9921% +3.7373%]
[ipc:bench]                         Performance has improved.
[ipc:bench] Found 3 outliers among 30 measurements (10.00%)
[ipc:bench]   2 (6.67%) low severe
[ipc:bench]   1 (3.33%) low mild
[ipc:bench]
[ipc:bench] Benchmarking encode_pipeline/64B
[ipc:bench] Benchmarking encode_pipeline/64B: Warming up for 3.0000 s
[ipc:bench] Benchmarking encode_pipeline/64B: Collecting 30 samples in estimated 30.000 s (51M iterations)
[ipc:bench] Benchmarking encode_pipeline/64B: Analyzing
[ipc:bench] encode_pipeline/64B     time:   [574.34 ns 578.07 ns 581.60 ns]
[ipc:bench]                         thrpt:  [104.94 MiB/s 105.58 MiB/s 106.27 MiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−0.7898% −0.2518% +0.2482%] (p = 0.33 > 0.05)
[ipc:bench]                         thrpt:  [−0.2476% +0.2525% +0.7961%]
[ipc:bench]                         No change in performance detected.
[ipc:bench] Found 1 outliers among 30 measurements (3.33%)
[ipc:bench]   1 (3.33%) low mild
[ipc:bench] Benchmarking encode_pipeline/1KiB
[ipc:bench] Benchmarking encode_pipeline/1KiB: Warming up for 3.0000 s
[ipc:bench] Benchmarking encode_pipeline/1KiB: Collecting 30 samples in estimated 30.000 s (16M iterations)
[ipc:bench] Benchmarking encode_pipeline/1KiB: Analyzing
[ipc:bench] encode_pipeline/1KiB    time:   [1.8216 µs 1.8311 µs 1.8438 µs]
[ipc:bench]                         thrpt:  [529.66 MiB/s 533.31 MiB/s 536.09 MiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [+0.9976% +1.5616% +2.2293%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [−2.1807% −1.5376% −0.9877%]
[ipc:bench]                         Change within noise threshold.
[ipc:bench] Found 3 outliers among 30 measurements (10.00%)
[ipc:bench]   1 (3.33%) high mild
[ipc:bench]   2 (6.67%) high severe
[ipc:bench] Benchmarking encode_pipeline/64KiB
[ipc:bench] Benchmarking encode_pipeline/64KiB: Warming up for 3.0000 s
[ipc:bench] Benchmarking encode_pipeline/64KiB: Collecting 30 samples in estimated 30.013 s (857k iterations)
[ipc:bench] Benchmarking encode_pipeline/64KiB: Analyzing
[ipc:bench] encode_pipeline/64KiB   time:   [35.232 µs 35.436 µs 35.591 µs]
[ipc:bench]                         thrpt:  [1.7149 GiB/s 1.7224 GiB/s 1.7324 GiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [+2.0162% +2.4505% +2.9052%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [−2.8232% −2.3919% −1.9763%]
[ipc:bench]                         Performance has regressed.
[ipc:bench] Benchmarking encode_pipeline/1MiB
[ipc:bench] Benchmarking encode_pipeline/1MiB: Warming up for 3.0000 s
[ipc:bench] Benchmarking encode_pipeline/1MiB: Collecting 30 samples in estimated 30.214 s (53k iterations)
[ipc:bench] Benchmarking encode_pipeline/1MiB: Analyzing
[ipc:bench] encode_pipeline/1MiB    time:   [570.17 µs 572.13 µs 574.06 µs]
[ipc:bench]                         thrpt:  [1.7012 GiB/s 1.7069 GiB/s 1.7128 GiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−0.9986% −0.4531% +0.0781%] (p = 0.10 > 0.05)
[ipc:bench]                         thrpt:  [−0.0780% +0.4551% +1.0087%]
[ipc:bench]                         No change in performance detected.
[ipc:bench] Found 1 outliers among 30 measurements (3.33%)
[ipc:bench]   1 (3.33%) high mild
[ipc:bench] Benchmarking encode_pipeline/~16MiB
[ipc:bench] Benchmarking encode_pipeline/~16MiB: Warming up for 3.0000 s
[ipc:bench] Benchmarking encode_pipeline/~16MiB: Collecting 30 samples in estimated 36.600 s (1395 iterations)
[ipc:bench] Benchmarking encode_pipeline/~16MiB: Analyzing
[ipc:bench] encode_pipeline/~16MiB  time:   [25.911 ms 26.190 ms 26.473 ms]
[ipc:bench]                         thrpt:  [604.38 MiB/s 610.92 MiB/s 617.50 MiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [+0.2770% +1.0010% +1.7205%] (p = 0.02 < 0.05)
[ipc:bench]                         thrpt:  [−1.6914% −0.9911% −0.2762%]
[ipc:bench]                         Change within noise threshold.
[ipc:bench]
[ipc:bench] Benchmarking decode_pipeline/64B
[ipc:bench] Benchmarking decode_pipeline/64B: Warming up for 3.0000 s
[ipc:bench] Benchmarking decode_pipeline/64B: Collecting 30 samples in estimated 30.000 s (75M iterations)
[ipc:bench] Benchmarking decode_pipeline/64B: Analyzing
[ipc:bench] decode_pipeline/64B     time:   [403.58 ns 404.45 ns 405.26 ns]
[ipc:bench]                         thrpt:  [150.61 MiB/s 150.91 MiB/s 151.24 MiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−0.1626% +0.2584% +0.6675%] (p = 0.24 > 0.05)
[ipc:bench]                         thrpt:  [−0.6631% −0.2577% +0.1628%]
[ipc:bench]                         No change in performance detected.
[ipc:bench] Found 1 outliers among 30 measurements (3.33%)
[ipc:bench]   1 (3.33%) high mild
[ipc:bench] Benchmarking decode_pipeline/1KiB
[ipc:bench] Benchmarking decode_pipeline/1KiB: Warming up for 3.0000 s
[ipc:bench] Benchmarking decode_pipeline/1KiB: Collecting 30 samples in estimated 30.000 s (43M iterations)
[ipc:bench] Benchmarking decode_pipeline/1KiB: Analyzing
[ipc:bench] decode_pipeline/1KiB    time:   [707.78 ns 713.13 ns 717.26 ns]
[ipc:bench]                         thrpt:  [1.3296 GiB/s 1.3373 GiB/s 1.3474 GiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [+1.6722% +2.3702% +3.0503%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [−2.9600% −2.3153% −1.6447%]
[ipc:bench]                         Performance has regressed.
[ipc:bench] Benchmarking decode_pipeline/64KiB
[ipc:bench] Benchmarking decode_pipeline/64KiB: Warming up for 3.0000 s
[ipc:bench] Benchmarking decode_pipeline/64KiB: Collecting 30 samples in estimated 30.002 s (2.0M iterations)
[ipc:bench] Benchmarking decode_pipeline/64KiB: Analyzing
[ipc:bench] decode_pipeline/64KiB   time:   [15.256 µs 15.352 µs 15.461 µs]
[ipc:bench]                         thrpt:  [3.9477 GiB/s 3.9757 GiB/s 4.0008 GiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [+1.9963% +2.7128% +3.4975%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [−3.3793% −2.6411% −1.9572%]
[ipc:bench]                         Performance has regressed.
[ipc:bench] Found 2 outliers among 30 measurements (6.67%)
[ipc:bench]   2 (6.67%) high mild
[ipc:bench] Benchmarking decode_pipeline/1MiB
[ipc:bench] Benchmarking decode_pipeline/1MiB: Warming up for 3.0000 s
[ipc:bench] Benchmarking decode_pipeline/1MiB: Collecting 30 samples in estimated 30.112 s (120k iterations)
[ipc:bench] Benchmarking decode_pipeline/1MiB: Analyzing
[ipc:bench] decode_pipeline/1MiB    time:   [248.59 µs 249.97 µs 251.33 µs]
[ipc:bench]                         thrpt:  [3.8856 GiB/s 3.9067 GiB/s 3.9285 GiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [+6.2434% +6.7605% +7.3547%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [−6.8508% −6.3324% −5.8765%]
[ipc:bench]                         Performance has regressed.
[ipc:bench] Found 1 outliers among 30 measurements (3.33%)
[ipc:bench]   1 (3.33%) high mild
[ipc:bench] Benchmarking decode_pipeline/~16MiB
[ipc:bench] Benchmarking decode_pipeline/~16MiB: Warming up for 3.0000 s
[ipc:bench] Benchmarking decode_pipeline/~16MiB: Collecting 30 samples in estimated 30.022 s (6975 iterations)
[ipc:bench] Benchmarking decode_pipeline/~16MiB: Analyzing
[ipc:bench] decode_pipeline/~16MiB  time:   [4.1797 ms 4.2249 ms 4.2708 ms]
[ipc:bench]                         thrpt:  [3.6585 GiB/s 3.6983 GiB/s 3.7383 GiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [+3.2482% +4.1816% +5.0552%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [−4.8120% −4.0138% −3.1460%]
[ipc:bench]                         Performance has regressed.
[ipc:bench]
[ipc:bench] Benchmarking encode_bulk_production/encode_16MiB
[ipc:bench] Benchmarking encode_bulk_production/encode_16MiB: Warming up for 3.0000 s
[ipc:bench] Benchmarking encode_bulk_production/encode_16MiB: Collecting 10 samples in estimated 61.081 s (2310 iterations)
[ipc:bench] Benchmarking encode_bulk_production/encode_16MiB: Analyzing
[ipc:bench] encode_bulk_production/encode_16MiB
[ipc:bench]                         time:   [25.410 ms 25.483 ms 25.578 ms]
[ipc:bench]                         change: [−2.0822% −1.4171% −0.6879%] (p = 0.00 < 0.05)
[ipc:bench]                         Change within noise threshold.
[ipc:bench] Found 1 outliers among 10 measurements (10.00%)
[ipc:bench]   1 (10.00%) high severe
[ipc:bench]
[ipc:bench] Benchmarking handshake/noise_ik
[ipc:bench] Benchmarking handshake/noise_ik: Warming up for 3.0000 s
[ipc:bench] Benchmarking handshake/noise_ik: Collecting 30 samples in estimated 30.067 s (52k iterations)
[ipc:bench] Benchmarking handshake/noise_ik: Analyzing
[ipc:bench] handshake/noise_ik      time:   [593.09 µs 599.39 µs 606.19 µs]
[ipc:bench]                         thrpt:  [1.6497 Kelem/s 1.6684 Kelem/s 1.6861 Kelem/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−0.2499% +0.7736% +1.8069%] (p = 0.16 > 0.05)
[ipc:bench]                         thrpt:  [−1.7748% −0.7677% +0.2506%]
[ipc:bench]                         No change in performance detected.
[ipc:bench] Found 1 outliers among 30 measurements (3.33%)
[ipc:bench]   1 (3.33%) high mild
[ipc:bench]
[ipc:bench] Benchmarking parallel_encrypt/1_workers
[ipc:bench] Benchmarking parallel_encrypt/1_workers: Warming up for 3.0000 s
[ipc:bench] Benchmarking parallel_encrypt/1_workers: Collecting 10 samples in estimated 60.144 s (13k iterations)
[ipc:bench] Benchmarking parallel_encrypt/1_workers: Analyzing
[ipc:bench] parallel_encrypt/1_workers
[ipc:bench]                         time:   [4.5651 ms 4.6230 ms 4.6759 ms]
[ipc:bench]                         thrpt:  [3.3416 GiB/s 3.3798 GiB/s 3.4227 GiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−3.5909% −2.6012% −1.6268%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [+1.6537% +2.6706% +3.7246%]
[ipc:bench]                         Performance has improved.
[ipc:bench] Found 2 outliers among 10 measurements (20.00%)
[ipc:bench]   1 (10.00%) low mild
[ipc:bench]   1 (10.00%) high mild
[ipc:bench] Benchmarking parallel_encrypt/2_workers
[ipc:bench] Benchmarking parallel_encrypt/2_workers: Warming up for 3.0000 s
[ipc:bench] Benchmarking parallel_encrypt/2_workers: Collecting 10 samples in estimated 60.079 s (12k iterations)
[ipc:bench] Benchmarking parallel_encrypt/2_workers: Analyzing
[ipc:bench] parallel_encrypt/2_workers
[ipc:bench]                         time:   [4.9953 ms 5.0405 ms 5.1152 ms]
[ipc:bench]                         thrpt:  [6.1092 GiB/s 6.1997 GiB/s 6.2559 GiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−2.5718% −1.2521% +0.0267%] (p = 0.11 > 0.05)
[ipc:bench]                         thrpt:  [−0.0267% +1.2679% +2.6397%]
[ipc:bench]                         No change in performance detected.
[ipc:bench] Benchmarking parallel_encrypt/4_workers
[ipc:bench] Benchmarking parallel_encrypt/4_workers: Warming up for 3.0000 s
[ipc:bench] Benchmarking parallel_encrypt/4_workers: Collecting 10 samples in estimated 60.233 s (9735 iterations)
[ipc:bench] Benchmarking parallel_encrypt/4_workers: Analyzing
[ipc:bench] parallel_encrypt/4_workers
[ipc:bench]                         time:   [6.2710 ms 6.4270 ms 6.5712 ms]
[ipc:bench]                         thrpt:  [9.5111 GiB/s 9.7246 GiB/s 9.9665 GiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−5.0408% −3.3675% −1.5977%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [+1.6237% +3.4848% +5.3084%]
[ipc:bench]                         Performance has improved.
[ipc:bench] Benchmarking parallel_encrypt/8_workers
[ipc:bench] Benchmarking parallel_encrypt/8_workers: Warming up for 3.0000 s
[ipc:bench] Benchmarking parallel_encrypt/8_workers: Collecting 10 samples in estimated 60.219 s (4565 iterations)
[ipc:bench] Benchmarking parallel_encrypt/8_workers: Analyzing
[ipc:bench] parallel_encrypt/8_workers
[ipc:bench]                         time:   [12.527 ms 13.078 ms 13.692 ms]
[ipc:bench]                         thrpt:  [9.1297 GiB/s 9.5579 GiB/s 9.9787 GiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−5.3256% −2.7920% +0.4050%] (p = 0.09 > 0.05)
[ipc:bench]                         thrpt:  [−0.4034% +2.8722% +5.6251%]
[ipc:bench]                         No change in performance detected.
[ipc:bench] Found 2 outliers among 10 measurements (20.00%)
[ipc:bench]   1 (10.00%) low mild
[ipc:bench]   1 (10.00%) high severe
[ipc:bench]
[ipc:bench] Benchmarking parallel_decrypt/1_workers
[ipc:bench] Benchmarking parallel_decrypt/1_workers: Warming up for 3.0000 s
[ipc:bench] Benchmarking parallel_decrypt/1_workers: Collecting 10 samples in estimated 60.180 s (14k iterations)
[ipc:bench] Benchmarking parallel_decrypt/1_workers: Analyzing
[ipc:bench] parallel_decrypt/1_workers
[ipc:bench]                         time:   [4.0997 ms 4.1397 ms 4.2132 ms]
[ipc:bench]                         thrpt:  [3.7085 GiB/s 3.7744 GiB/s 3.8112 GiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−1.9089% −0.4606% +1.2027%] (p = 0.61 > 0.05)
[ipc:bench]                         thrpt:  [−1.1884% +0.4628% +1.9460%]
[ipc:bench]                         No change in performance detected.
[ipc:bench] Benchmarking parallel_decrypt/2_workers
[ipc:bench] Benchmarking parallel_decrypt/2_workers: Warming up for 3.0000 s
[ipc:bench] Benchmarking parallel_decrypt/2_workers: Collecting 10 samples in estimated 60.039 s (13k iterations)
[ipc:bench] Benchmarking parallel_decrypt/2_workers: Analyzing
[ipc:bench] parallel_decrypt/2_workers
[ipc:bench]                         time:   [4.5948 ms 4.6192 ms 4.6434 ms]
[ipc:bench]                         thrpt:  [6.7300 GiB/s 6.7652 GiB/s 6.8011 GiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−2.5530% −2.0808% −1.6784%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [+1.7071% +2.1251% +2.6199%]
[ipc:bench]                         Performance has improved.
[ipc:bench] Benchmarking parallel_decrypt/4_workers
[ipc:bench] Benchmarking parallel_decrypt/4_workers: Warming up for 3.0000 s
[ipc:bench] Benchmarking parallel_decrypt/4_workers: Collecting 10 samples in estimated 60.032 s (9460 iterations)
[ipc:bench] Benchmarking parallel_decrypt/4_workers: Analyzing
[ipc:bench] parallel_decrypt/4_workers
[ipc:bench]                         time:   [6.0618 ms 6.0929 ms 6.1332 ms]
[ipc:bench]                         thrpt:  [10.190 GiB/s 10.258 GiB/s 10.310 GiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−5.3472% −4.8661% −4.3303%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [+4.5263% +5.1150% +5.6493%]
[ipc:bench]                         Performance has improved.
[ipc:bench] Benchmarking parallel_decrypt/8_workers
[ipc:bench] Benchmarking parallel_decrypt/8_workers: Warming up for 3.0000 s
[ipc:bench] Benchmarking parallel_decrypt/8_workers: Collecting 10 samples in estimated 60.294 s (4950 iterations)
[ipc:bench] Benchmarking parallel_decrypt/8_workers: Analyzing
[ipc:bench] parallel_decrypt/8_workers
[ipc:bench]                         time:   [12.224 ms 12.524 ms 12.703 ms]
[ipc:bench]                         thrpt:  [9.8401 GiB/s 9.9809 GiB/s 10.226 GiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−8.0827% −6.2433% −4.5199%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [+4.7339% +6.6590% +8.7934%]
[ipc:bench]                         Performance has improved.
[ipc:bench]
[ipc:bench] Benchmarking parallel_encode/1_workers
[ipc:bench] Benchmarking parallel_encode/1_workers: Warming up for 3.0000 s
[ipc:bench] Benchmarking parallel_encode/1_workers: Collecting 10 samples in estimated 60.807 s (2200 iterations)
[ipc:bench] Benchmarking parallel_encode/1_workers: Analyzing
[ipc:bench] parallel_encode/1_workers
[ipc:bench]                         time:   [27.480 ms 27.799 ms 28.104 ms]
[ipc:bench]                         thrpt:  [569.32 MiB/s 575.57 MiB/s 582.25 MiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−4.0404% −3.2403% −2.3251%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [+2.3804% +3.3489% +4.2105%]
[ipc:bench]                         Performance has improved.
[ipc:bench] Benchmarking parallel_encode/2_workers
[ipc:bench] Benchmarking parallel_encode/2_workers: Warming up for 3.0000 s
[ipc:bench] Benchmarking parallel_encode/2_workers: Collecting 10 samples in estimated 60.772 s (2035 iterations)
[ipc:bench] Benchmarking parallel_encode/2_workers: Analyzing
[ipc:bench] parallel_encode/2_workers
[ipc:bench]                         time:   [29.457 ms 29.876 ms 30.132 ms]
[ipc:bench]                         thrpt:  [1.0371 GiB/s 1.0460 GiB/s 1.0609 GiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−5.1856% −4.4356% −3.7058%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [+3.8484% +4.6415% +5.4692%]
[ipc:bench]                         Performance has improved.
[ipc:bench] Benchmarking parallel_encode/4_workers
[ipc:bench] Benchmarking parallel_encode/4_workers: Warming up for 3.0000 s
[ipc:bench] Benchmarking parallel_encode/4_workers: Collecting 10 samples in estimated 60.700 s (1155 iterations)
[ipc:bench] Benchmarking parallel_encode/4_workers: Analyzing
[ipc:bench] parallel_encode/4_workers
[ipc:bench]                         time:   [52.302 ms 54.155 ms 55.184 ms]
[ipc:bench]                         thrpt:  [1.1326 GiB/s 1.1541 GiB/s 1.1950 GiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−2.0416% −0.0279% +2.2707%] (p = 0.97 > 0.05)
[ipc:bench]                         thrpt:  [−2.2203% +0.0279% +2.0842%]
[ipc:bench]                         No change in performance detected.
[ipc:bench] Benchmarking parallel_encode/8_workers
[ipc:bench] Benchmarking parallel_encode/8_workers: Warming up for 3.0000 s
[ipc:bench] Benchmarking parallel_encode/8_workers: Collecting 10 samples in estimated 62.911 s (715 iterations)
[ipc:bench] Benchmarking parallel_encode/8_workers: Analyzing
[ipc:bench] parallel_encode/8_workers
[ipc:bench]                         time:   [86.300 ms 87.349 ms 88.197 ms]
[ipc:bench]                         thrpt:  [1.4173 GiB/s 1.4310 GiB/s 1.4484 GiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [+3.1394% +4.7605% +6.2904%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [−5.9182% −4.5441% −3.0438%]
[ipc:bench]                         Performance has regressed.
[ipc:bench]
[ipc:bench]     Finished `bench` profile [optimized] target(s) in 0.26s
[ipc:bench]      Running benches/v3/socket_e2e.rs (target/release/deps/v3_socket_e2e-0020ef389e56f45b)
[ipc:bench] Benchmarking storage_crud/resolve_friend_name_req
[ipc:bench] Benchmarking storage_crud/resolve_friend_name_req: Warming up for 3.0000 s
[ipc:bench] Benchmarking storage_crud/resolve_friend_name_req: Collecting 20 samples in estimated 30.008 s (653k iterations)
[ipc:bench] Benchmarking storage_crud/resolve_friend_name_req: Analyzing
[ipc:bench] storage_crud/resolve_friend_name_req
[ipc:bench]                         time:   [45.899 µs 46.252 µs 46.510 µs]
[ipc:bench]                         thrpt:  [1.6404 MiB/s 1.6495 MiB/s 1.6622 MiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [+7.4804% +9.2552% +11.385%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [−10.221% −8.4712% −6.9598%]
[ipc:bench]                         Performance has regressed.
[ipc:bench] Found 2 outliers among 20 measurements (10.00%)
[ipc:bench]   1 (5.00%) low mild
[ipc:bench]   1 (5.00%) high severe
[ipc:bench] Benchmarking storage_crud/resolve_friend_name_reply
[ipc:bench] Benchmarking storage_crud/resolve_friend_name_reply: Warming up for 3.0000 s
[ipc:bench] Benchmarking storage_crud/resolve_friend_name_reply: Collecting 20 samples in estimated 30.010 s (628k iterations)
[ipc:bench] Benchmarking storage_crud/resolve_friend_name_reply: Analyzing
[ipc:bench] storage_crud/resolve_friend_name_reply
[ipc:bench]                         time:   [46.721 µs 46.999 µs 47.363 µs]
[ipc:bench]                         thrpt:  [2.8190 MiB/s 2.8408 MiB/s 2.8577 MiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [+8.1673% +9.8979% +11.370%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [−10.209% −9.0064% −7.5506%]
[ipc:bench]                         Performance has regressed.
[ipc:bench] Found 1 outliers among 20 measurements (5.00%)
[ipc:bench]   1 (5.00%) high mild
[ipc:bench] Benchmarking storage_crud/delete_key_req
[ipc:bench] Benchmarking storage_crud/delete_key_req: Warming up for 3.0000 s
[ipc:bench] Benchmarking storage_crud/delete_key_req: Collecting 20 samples in estimated 30.006 s (644k iterations)
[ipc:bench] Benchmarking storage_crud/delete_key_req: Analyzing
[ipc:bench] storage_crud/delete_key_req
[ipc:bench]                         time:   [47.027 µs 47.352 µs 47.637 µs]
[ipc:bench]                         thrpt:  [2.7227 MiB/s 2.7390 MiB/s 2.7580 MiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [+9.5402% +11.112% +12.894%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [−11.421% −10.001% −8.7093%]
[ipc:bench]                         Performance has regressed.
[ipc:bench] Found 1 outliers among 20 measurements (5.00%)
[ipc:bench]   1 (5.00%) high severe
[ipc:bench] Benchmarking storage_crud/store_friend_name_req
[ipc:bench] Benchmarking storage_crud/store_friend_name_req: Warming up for 3.0000 s
[ipc:bench] Benchmarking storage_crud/store_friend_name_req: Collecting 20 samples in estimated 30.001 s (611k iterations)
[ipc:bench] Benchmarking storage_crud/store_friend_name_req: Analyzing
[ipc:bench] storage_crud/store_friend_name_req
[ipc:bench]                         time:   [47.601 µs 47.848 µs 48.069 µs]
[ipc:bench]                         thrpt:  [3.9679 MiB/s 3.9863 MiB/s 4.0070 MiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [+9.4396% +10.801% +12.361%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [−11.001% −9.7479% −8.6254%]
[ipc:bench]                         Performance has regressed.
[ipc:bench] Found 6 outliers among 20 measurements (30.00%)
[ipc:bench]   3 (15.00%) low mild
[ipc:bench]   3 (15.00%) high mild
[ipc:bench] Benchmarking storage_crud/count_keys_req
[ipc:bench] Benchmarking storage_crud/count_keys_req: Warming up for 3.0000 s
[ipc:bench] Benchmarking storage_crud/count_keys_req: Collecting 20 samples in estimated 30.008 s (601k iterations)
[ipc:bench] Benchmarking storage_crud/count_keys_req: Analyzing
[ipc:bench] storage_crud/count_keys_req
[ipc:bench]                         time:   [46.399 µs 46.676 µs 46.962 µs]
[ipc:bench]                         thrpt:  [83.179 KiB/s 83.689 KiB/s 84.188 KiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [+9.4435% +11.171% +12.914%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [−11.437% −10.049% −8.6286%]
[ipc:bench]                         Performance has regressed.
[ipc:bench] Found 3 outliers among 20 measurements (15.00%)
[ipc:bench]   2 (10.00%) low mild
[ipc:bench]   1 (5.00%) high mild
[ipc:bench] Benchmarking storage_crud/count_keys_reply
[ipc:bench] Benchmarking storage_crud/count_keys_reply: Warming up for 3.0000 s
[ipc:bench] Benchmarking storage_crud/count_keys_reply: Collecting 20 samples in estimated 30.001 s (639k iterations)
[ipc:bench] Benchmarking storage_crud/count_keys_reply: Analyzing
[ipc:bench] storage_crud/count_keys_reply
[ipc:bench]                         time:   [45.853 µs 46.307 µs 46.813 µs]
[ipc:bench]                         thrpt:  [250.33 KiB/s 253.06 KiB/s 255.57 KiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [+10.125% +12.340% +14.494%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [−12.659% −10.985% −9.1940%]
[ipc:bench]                         Performance has regressed.
[ipc:bench] Found 3 outliers among 20 measurements (15.00%)
[ipc:bench]   1 (5.00%) low severe
[ipc:bench]   2 (10.00%) low mild
[ipc:bench]
[ipc:bench] Benchmarking storage_batch/10_ops
[ipc:bench] Benchmarking storage_batch/10_ops: Warming up for 3.0000 s
[ipc:bench] Benchmarking storage_batch/10_ops: Collecting 20 samples in estimated 30.007 s (573k iterations)
[ipc:bench] Benchmarking storage_batch/10_ops: Analyzing
[ipc:bench] storage_batch/10_ops    time:   [51.114 µs 51.519 µs 52.012 µs]
[ipc:bench]                         thrpt:  [14.669 MiB/s 14.809 MiB/s 14.926 MiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [+11.230% +12.538% +13.857%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [−12.170% −11.141% −10.096%]
[ipc:bench]                         Performance has regressed.
[ipc:bench] Benchmarking storage_batch/50_ops
[ipc:bench] Benchmarking storage_batch/50_ops: Warming up for 3.0000 s
[ipc:bench] Benchmarking storage_batch/50_ops: Collecting 20 samples in estimated 30.014 s (428k iterations)
[ipc:bench] Benchmarking storage_batch/50_ops: Analyzing
[ipc:bench] storage_batch/50_ops    time:   [67.577 µs 67.995 µs 68.351 µs]
[ipc:bench]                         thrpt:  [55.810 MiB/s 56.103 MiB/s 56.450 MiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [+11.065% +12.020% +13.047%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [−11.541% −10.730% −9.9623%]
[ipc:bench]                         Performance has regressed.
[ipc:bench] Found 2 outliers among 20 measurements (10.00%)
[ipc:bench]   2 (10.00%) high mild
[ipc:bench] Benchmarking storage_batch/200_ops
[ipc:bench] Benchmarking storage_batch/200_ops: Warming up for 3.0000 s
[ipc:bench] Benchmarking storage_batch/200_ops: Collecting 20 samples in estimated 30.007 s (253k iterations)
[ipc:bench] Benchmarking storage_batch/200_ops: Analyzing
[ipc:bench] storage_batch/200_ops   time:   [112.71 µs 114.15 µs 115.47 µs]
[ipc:bench]                         thrpt:  [132.15 MiB/s 133.67 MiB/s 135.38 MiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [+10.372% +11.785% +13.303%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [−11.741% −10.543% −9.3977%]
[ipc:bench]                         Performance has regressed.
[ipc:bench]
[ipc:bench] Benchmarking chat_sustained/100_sequential_requests
[ipc:bench] Benchmarking chat_sustained/100_sequential_requests: Warming up for 3.0000 s
[ipc:bench] Benchmarking chat_sustained/100_sequential_requests: Collecting 20 samples in estimated 30.775 s (6300 iterations)
[ipc:bench] Benchmarking chat_sustained/100_sequential_requests: Analyzing
[ipc:bench] chat_sustained/100_sequential_requests
[ipc:bench]                         time:   [4.7506 ms 4.8093 ms 4.8658 ms]
[ipc:bench]                         thrpt:  [20.552 Kelem/s 20.793 Kelem/s 21.050 Kelem/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [+9.8737% +11.355% +12.996%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [−11.501% −10.197% −8.9864%]
[ipc:bench]                         Performance has regressed.
[ipc:bench]
[ipc:bench] Benchmarking attachment_throughput/1KiB_tiny_image
[ipc:bench] Benchmarking attachment_throughput/1KiB_tiny_image: Warming up for 3.0000 s
[ipc:bench] Benchmarking attachment_throughput/1KiB_tiny_image: Collecting 10 samples in estimated 60.001 s (1.1M iterations)
[ipc:bench] Benchmarking attachment_throughput/1KiB_tiny_image: Analyzing
[ipc:bench] attachment_throughput/1KiB_tiny_image
[ipc:bench]                         time:   [53.702 µs 54.117 µs 54.388 µs]
[ipc:bench]                         thrpt:  [17.955 MiB/s 18.045 MiB/s 18.185 MiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [+10.284% +11.747% +13.256%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [−11.704% −10.512% −9.3249%]
[ipc:bench]                         Performance has regressed.
[ipc:bench] Benchmarking attachment_throughput/100KiB_photo_thumb
[ipc:bench] Benchmarking attachment_throughput/100KiB_photo_thumb: Warming up for 3.0000 s
[ipc:bench] Benchmarking attachment_throughput/100KiB_photo_thumb: Collecting 10 samples in estimated 60.008 s (227k iterations)
[ipc:bench] Benchmarking attachment_throughput/100KiB_photo_thumb: Analyzing
[ipc:bench] attachment_throughput/100KiB_photo_thumb
[ipc:bench]                         time:   [230.56 µs 236.27 µs 247.33 µs]
[ipc:bench]                         thrpt:  [394.85 MiB/s 413.33 MiB/s 423.56 MiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [+4.8306% +7.6957% +10.722%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [−9.6840% −7.1458% −4.6080%]
[ipc:bench]                         Performance has regressed.
[ipc:bench] Benchmarking attachment_throughput/1MiB_document
[ipc:bench] Benchmarking attachment_throughput/1MiB_document: Warming up for 3.0000 s
[ipc:bench] Benchmarking attachment_throughput/1MiB_document: Collecting 10 samples in estimated 60.006 s (39k iterations)
[ipc:bench] Benchmarking attachment_throughput/1MiB_document: Analyzing
[ipc:bench] attachment_throughput/1MiB_document
[ipc:bench]                         time:   [1.5652 ms 1.5743 ms 1.5842 ms]
[ipc:bench]                         thrpt:  [631.25 MiB/s 635.19 MiB/s 638.88 MiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [+2.4811% +3.2393% +4.0266%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [−3.8707% −3.1376% −2.4210%]
[ipc:bench]                         Performance has regressed.
[ipc:bench] Benchmarking attachment_throughput/10MiB_photo
[ipc:bench] Benchmarking attachment_throughput/10MiB_photo: Warming up for 3.0000 s
[ipc:bench] Benchmarking attachment_throughput/10MiB_photo: Collecting 10 samples in estimated 60.723 s (3630 iterations)
[ipc:bench] Benchmarking attachment_throughput/10MiB_photo: Analyzing
[ipc:bench] attachment_throughput/10MiB_photo
[ipc:bench]                         time:   [16.735 ms 16.860 ms 16.957 ms]
[ipc:bench]                         thrpt:  [589.73 MiB/s 593.11 MiB/s 597.54 MiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [+0.7948% +1.5272% +2.3353%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [−2.2820% −1.5042% −0.7885%]
[ipc:bench]                         Change within noise threshold.
[ipc:bench] Benchmarking attachment_throughput/64MiB_video
[ipc:bench] Benchmarking attachment_throughput/64MiB_video: Warming up for 3.0000 s
[ipc:bench] Benchmarking attachment_throughput/64MiB_video: Collecting 10 samples in estimated 61.763 s (990 iterations)
[ipc:bench] Benchmarking attachment_throughput/64MiB_video: Analyzing
[ipc:bench] attachment_throughput/64MiB_video
[ipc:bench]                         time:   [60.607 ms 60.866 ms 61.402 ms]
[ipc:bench]                         thrpt:  [1.0179 GiB/s 1.0268 GiB/s 1.0312 GiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [+1.0062% +2.4526% +4.3424%] (p = 0.01 < 0.05)
[ipc:bench]                         thrpt:  [−4.1617% −2.3939% −0.9962%]
[ipc:bench]                         Performance has regressed.
[ipc:bench]
[ipc:bench] Benchmarking mixed_workload/10MiB_upload_plus_50_messages
[ipc:bench] Benchmarking mixed_workload/10MiB_upload_plus_50_messages: Warming up for 3.0000 s
[ipc:bench] Benchmarking mixed_workload/10MiB_upload_plus_50_messages: Collecting 10 samples in estimated 60.026 s (3465 iterations)
[ipc:bench] Benchmarking mixed_workload/10MiB_upload_plus_50_messages: Analyzing
[ipc:bench] mixed_workload/10MiB_upload_plus_50_messages
[ipc:bench]                         time:   [16.602 ms 16.703 ms 16.845 ms]
[ipc:bench]                         thrpt:  [594.38 MiB/s 599.42 MiB/s 603.07 MiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−1.4954% −0.2783% +0.8002%] (p = 0.66 > 0.05)
[ipc:bench]                         thrpt:  [−0.7938% +0.2791% +1.5181%]
[ipc:bench]                         No change in performance detected.
[ipc:bench]
[ipc:bench] Benchmarking escalation_boundary/32KiB_inline
[ipc:bench] Benchmarking escalation_boundary/32KiB_inline: Warming up for 3.0000 s
[ipc:bench] Benchmarking escalation_boundary/32KiB_inline: Collecting 20 samples in estimated 30.025 s (252k iterations)
[ipc:bench] Benchmarking escalation_boundary/32KiB_inline: Analyzing
[ipc:bench] escalation_boundary/32KiB_inline
[ipc:bench]                         time:   [120.09 µs 121.33 µs 122.62 µs]
[ipc:bench]                         thrpt:  [254.84 MiB/s 257.56 MiB/s 260.22 MiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−4.4071% −2.7306% −0.9641%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [+0.9735% +2.8073% +4.6103%]
[ipc:bench]                         Change within noise threshold.
[ipc:bench] Found 1 outliers among 20 measurements (5.00%)
[ipc:bench]   1 (5.00%) high mild
[ipc:bench] Benchmarking escalation_boundary/63KiB_inline
[ipc:bench] Benchmarking escalation_boundary/63KiB_inline: Warming up for 3.0000 s
[ipc:bench] Benchmarking escalation_boundary/63KiB_inline: Collecting 20 samples in estimated 30.011 s (176k iterations)
[ipc:bench] Benchmarking escalation_boundary/63KiB_inline: Analyzing
[ipc:bench] escalation_boundary/63KiB_inline
[ipc:bench]                         time:   [171.09 µs 173.76 µs 176.29 µs]
[ipc:bench]                         thrpt:  [348.98 MiB/s 354.07 MiB/s 359.60 MiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−4.7532% −2.8735% −1.0628%] (p = 0.01 < 0.05)
[ipc:bench]                         thrpt:  [+1.0742% +2.9585% +4.9904%]
[ipc:bench]                         Performance has improved.
[ipc:bench] Found 1 outliers among 20 measurements (5.00%)
[ipc:bench]   1 (5.00%) high mild
[ipc:bench] Benchmarking escalation_boundary/65KiB_bulk
[ipc:bench] Benchmarking escalation_boundary/65KiB_bulk: Warming up for 3.0000 s
[ipc:bench] Benchmarking escalation_boundary/65KiB_bulk: Collecting 20 samples in estimated 30.025 s (173k iterations)
[ipc:bench] Benchmarking escalation_boundary/65KiB_bulk: Analyzing
[ipc:bench] escalation_boundary/65KiB_bulk
[ipc:bench]                         time:   [169.78 µs 171.54 µs 173.71 µs]
[ipc:bench]                         thrpt:  [365.43 MiB/s 370.04 MiB/s 373.88 MiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−3.0933% −1.9167% −0.5967%] (p = 0.01 < 0.05)
[ipc:bench]                         thrpt:  [+0.6003% +1.9541% +3.1920%]
[ipc:bench]                         Change within noise threshold.
[ipc:bench] Benchmarking escalation_boundary/128KiB_bulk
[ipc:bench] Benchmarking escalation_boundary/128KiB_bulk: Warming up for 3.0000 s
[ipc:bench] Benchmarking escalation_boundary/128KiB_bulk: Collecting 20 samples in estimated 30.026 s (119k iterations)
[ipc:bench] Benchmarking escalation_boundary/128KiB_bulk: Analyzing
[ipc:bench] escalation_boundary/128KiB_bulk
[ipc:bench]                         time:   [252.94 µs 255.60 µs 258.31 µs]
[ipc:bench]                         thrpt:  [483.92 MiB/s 489.04 MiB/s 494.19 MiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−2.5935% −1.5477% −0.5369%] (p = 0.01 < 0.05)
[ipc:bench]                         thrpt:  [+0.5398% +1.5721% +2.6626%]
[ipc:bench]                         Change within noise threshold.
[ipc:bench] Benchmarking escalation_boundary/256KiB_bulk
[ipc:bench] Benchmarking escalation_boundary/256KiB_bulk: Warming up for 3.0000 s
[ipc:bench] Benchmarking escalation_boundary/256KiB_bulk: Collecting 20 samples in estimated 30.059 s (72k iterations)
[ipc:bench] Benchmarking escalation_boundary/256KiB_bulk: Analyzing
[ipc:bench] escalation_boundary/256KiB_bulk
[ipc:bench]                         time:   [420.87 µs 424.76 µs 430.37 µs]
[ipc:bench]                         thrpt:  [580.90 MiB/s 588.56 MiB/s 594.01 MiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−3.1125% −2.4027% −1.6073%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [+1.6336% +2.4618% +3.2124%]
[ipc:bench]                         Performance has improved.
[ipc:bench] Found 1 outliers among 20 measurements (5.00%)
[ipc:bench]   1 (5.00%) high mild
[ipc:bench]
[ipc:bench] Benchmarking cold_start/connect_first_request_shutdown
[ipc:bench] Benchmarking cold_start/connect_first_request_shutdown: Warming up for 3.0000 s
[ipc:bench] Benchmarking cold_start/connect_first_request_shutdown: Collecting 10 samples in estimated 30.043 s (24k iterations)
[ipc:bench] Benchmarking cold_start/connect_first_request_shutdown: Analyzing
[ipc:bench] cold_start/connect_first_request_shutdown
[ipc:bench]                         time:   [1.2416 ms 1.2567 ms 1.2863 ms]
[ipc:bench]                         change: [−2.5120% −0.7159% +1.2984%] (p = 0.52 > 0.05)
[ipc:bench]                         No change in performance detected.
[ipc:bench]
[ipc:bench] Benchmarking concurrent_bulk/2_streams
[ipc:bench] Benchmarking concurrent_bulk/2_streams: Warming up for 3.0000 s
[ipc:bench] Benchmarking concurrent_bulk/2_streams: Collecting 10 samples in estimated 61.414 s (1705 iterations)
[ipc:bench] Benchmarking concurrent_bulk/2_streams: Analyzing
[ipc:bench] concurrent_bulk/2_streams
[ipc:bench]                         time:   [36.024 ms 36.269 ms 36.565 ms]
[ipc:bench]                         thrpt:  [875.16 MiB/s 882.29 MiB/s 888.29 MiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−3.1823% −2.6277% −2.0221%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [+2.0638% +2.6986% +3.2869%]
[ipc:bench]                         Performance has improved.
[ipc:bench] Found 1 outliers among 10 measurements (10.00%)
[ipc:bench]   1 (10.00%) high mild
[ipc:bench] Benchmarking concurrent_bulk/8_streams
[ipc:bench] Benchmarking concurrent_bulk/8_streams: Warming up for 3.0000 s
[ipc:bench] Benchmarking concurrent_bulk/8_streams: Collecting 10 samples in estimated 63.731 s (605 iterations)
[ipc:bench] Benchmarking concurrent_bulk/8_streams: Analyzing
[ipc:bench] concurrent_bulk/8_streams
[ipc:bench]                         time:   [105.33 ms 106.15 ms 107.32 ms]
[ipc:bench]                         thrpt:  [1.1647 GiB/s 1.1776 GiB/s 1.1867 GiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−5.4374% −4.6191% −3.7472%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [+3.8931% +4.8428% +5.7500%]
[ipc:bench]                         Performance has improved.
[ipc:bench] Found 1 outliers among 10 measurements (10.00%)
[ipc:bench]   1 (10.00%) high mild
[ipc:bench]
[ipc:bench] Benchmarking notify_throughput/1000_fire_and_forget
[ipc:bench] Benchmarking notify_throughput/1000_fire_and_forget: Warming up for 3.0000 s
[ipc:bench] Benchmarking notify_throughput/1000_fire_and_forget: Collecting 20 samples in estimated 30.058 s (40k iterations)
[ipc:bench] Benchmarking notify_throughput/1000_fire_and_forget: Analyzing
[ipc:bench] notify_throughput/1000_fire_and_forget
[ipc:bench]                         time:   [442.68 µs 445.73 µs 450.08 µs]
[ipc:bench]                         thrpt:  [2.2219 Melem/s 2.2435 Melem/s 2.2590 Melem/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−3.2923% −2.6442% −1.9983%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [+2.0390% +2.7160% +3.4043%]
[ipc:bench]                         Performance has improved.
[ipc:bench] Found 1 outliers among 20 measurements (5.00%)
[ipc:bench]   1 (5.00%) high mild
[ipc:bench]
[ipc:bench] Benchmarking multi_client/1_clients
[ipc:bench] Benchmarking multi_client/1_clients: Warming up for 3.0000 s
[ipc:bench] Benchmarking multi_client/1_clients: Collecting 20 samples in estimated 30.029 s (71k iterations)
[ipc:bench] Benchmarking multi_client/1_clients: Analyzing
[ipc:bench] multi_client/1_clients  time:   [415.34 µs 420.24 µs 425.54 µs]
[ipc:bench]                         thrpt:  [23.499 Kelem/s 23.796 Kelem/s 24.076 Kelem/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−5.4220% −3.8914% −2.2178%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [+2.2681% +4.0490% +5.7328%]
[ipc:bench]                         Performance has improved.
[ipc:bench] Benchmarking multi_client/2_clients
[ipc:bench] Benchmarking multi_client/2_clients: Warming up for 3.0000 s
[ipc:bench] Benchmarking multi_client/2_clients: Collecting 20 samples in estimated 30.066 s (60k iterations)
[ipc:bench] Benchmarking multi_client/2_clients: Analyzing
[ipc:bench] multi_client/2_clients  time:   [475.26 µs 477.28 µs 479.01 µs]
[ipc:bench]                         thrpt:  [41.753 Kelem/s 41.904 Kelem/s 42.083 Kelem/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−5.0626% −3.9329% −2.3858%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [+2.4441% +4.0939% +5.3326%]
[ipc:bench]                         Performance has improved.
[ipc:bench] Found 2 outliers among 20 measurements (10.00%)
[ipc:bench]   1 (5.00%) low mild
[ipc:bench]   1 (5.00%) high severe
[ipc:bench] Benchmarking multi_client/8_clients
[ipc:bench] Benchmarking multi_client/8_clients: Warming up for 3.0000 s
[ipc:bench] Benchmarking multi_client/8_clients: Collecting 20 samples in estimated 30.048 s (35k iterations)
[ipc:bench] Benchmarking multi_client/8_clients: Analyzing
[ipc:bench] multi_client/8_clients  time:   [868.56 µs 887.86 µs 899.82 µs]
[ipc:bench]                         thrpt:  [88.907 Kelem/s 90.105 Kelem/s 92.106 Kelem/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−11.577% −9.9422% −8.4331%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [+9.2097% +11.040% +13.093%]
[ipc:bench]                         Performance has improved.
[ipc:bench]
[ipc:bench] Benchmarking sustained_throughput/16MiB_sustained
[ipc:bench] Benchmarking sustained_throughput/16MiB_sustained: Warming up for 3.0000 s
[ipc:bench] Benchmarking sustained_throughput/16MiB_sustained: Collecting 10 samples in estimated 61.266 s (2310 iterations)
[ipc:bench] Benchmarking sustained_throughput/16MiB_sustained: Analyzing
[ipc:bench] sustained_throughput/16MiB_sustained
[ipc:bench]                         time:   [26.323 ms 26.461 ms 26.551 ms]
[ipc:bench]                         thrpt:  [602.60 MiB/s 604.66 MiB/s 607.84 MiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−1.8886% −1.4193% −0.9550%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [+0.9642% +1.4398% +1.9249%]
[ipc:bench]                         Change within noise threshold.
[ipc:bench]
[ipc:bench] Benchmarking connection_density/100_idle_connections
[ipc:bench] Benchmarking connection_density/100_idle_connections: Warming up for 3.0000 s
[ipc:bench] Benchmarking connection_density/100_idle_connections: Collecting 10 samples in estimated 31.472 s (220 iterations)
[ipc:bench] Benchmarking connection_density/100_idle_connections: Analyzing
[ipc:bench] connection_density/100_idle_connections
[ipc:bench]                         time:   [140.27 ms 146.01 ms 149.80 ms]
[ipc:bench]                         change: [−13.673% −8.8179% −3.4568%] (p = 0.01 < 0.05)
[ipc:bench]                         Performance has improved.
[ipc:bench]
[ipc:bench] Benchmarking memory_stability/500_transfers_sustained
[ipc:bench] Benchmarking memory_stability/500_transfers_sustained: Warming up for 3.0000 s
[ipc:bench] Benchmarking memory_stability/500_transfers_sustained: Collecting 10 samples in estimated 60.358 s (715 iterations)
[ipc:bench] Benchmarking memory_stability/500_transfers_sustained: Analyzing
[ipc:bench] memory_stability/500_transfers_sustained
[ipc:bench]                         time:   [81.948 ms 82.758 ms 83.542 ms]
[ipc:bench]                         change: [−5.1914% −3.8392% −2.5039%] (p = 0.00 < 0.05)
[ipc:bench]                         Performance has improved.
[ipc:bench]
[ipc:bench] Benchmarking reconnect_under_load/connect_request_disconnect_while_busy
[ipc:bench] Benchmarking reconnect_under_load/connect_request_disconnect_while_busy: Warming up for 3.0000 s
[ipc:bench] Benchmarking reconnect_under_load/connect_request_disconnect_while_busy: Collecting 10 samples in estimated 30.008 s (22k iterations)
[ipc:bench] Benchmarking reconnect_under_load/connect_request_disconnect_while_busy: Analyzing
[ipc:bench] reconnect_under_load/connect_request_disconnect_while_busy
[ipc:bench]                         time:   [1.3520 ms 1.3716 ms 1.3947 ms]
[ipc:bench]                         change: [−6.5104% −4.8578% −3.2712%] (p = 0.00 < 0.05)
[ipc:bench]                         Performance has improved.
[ipc:bench] Found 1 outliers among 10 measurements (10.00%)
[ipc:bench]   1 (10.00%) high mild
[ipc:bench]
[ipc:bench] Benchmarking server_to_client_bulk/1KiB
[ipc:bench] Benchmarking server_to_client_bulk/1KiB: Warming up for 3.0000 s
[ipc:bench] Benchmarking server_to_client_bulk/1KiB: Collecting 10 samples in estimated 60.003 s (1.2M iterations)
[ipc:bench] Benchmarking server_to_client_bulk/1KiB: Analyzing
[ipc:bench] server_to_client_bulk/1KiB
[ipc:bench]                         time:   [52.099 µs 52.611 µs 53.270 µs]
[ipc:bench]                         thrpt:  [18.332 MiB/s 18.562 MiB/s 18.745 MiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−4.5487% −2.6980% −1.0758%] (p = 0.01 < 0.05)
[ipc:bench]                         thrpt:  [+1.0875% +2.7729% +4.7655%]
[ipc:bench]                         Performance has improved.
[ipc:bench] Found 1 outliers among 10 measurements (10.00%)
[ipc:bench]   1 (10.00%) low mild
[ipc:bench] Benchmarking server_to_client_bulk/64KiB
[ipc:bench] Benchmarking server_to_client_bulk/64KiB: Warming up for 3.0000 s
[ipc:bench] Benchmarking server_to_client_bulk/64KiB: Collecting 10 samples in estimated 60.003 s (385k iterations)
[ipc:bench] Benchmarking server_to_client_bulk/64KiB: Analyzing
[ipc:bench] server_to_client_bulk/64KiB
[ipc:bench]                         time:   [152.86 µs 154.06 µs 154.74 µs]
[ipc:bench]                         thrpt:  [403.90 MiB/s 405.69 MiB/s 408.88 MiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−2.8364% −1.8339% −0.5744%] (p = 0.01 < 0.05)
[ipc:bench]                         thrpt:  [+0.5777% +1.8682% +2.9192%]
[ipc:bench]                         Change within noise threshold.
[ipc:bench] Found 1 outliers among 10 measurements (10.00%)
[ipc:bench]   1 (10.00%) high severe
[ipc:bench] Benchmarking server_to_client_bulk/1MiB
[ipc:bench] Benchmarking server_to_client_bulk/1MiB: Warming up for 3.0000 s
[ipc:bench] Benchmarking server_to_client_bulk/1MiB: Collecting 10 samples in estimated 60.006 s (38k iterations)
[ipc:bench] Benchmarking server_to_client_bulk/1MiB: Analyzing
[ipc:bench] server_to_client_bulk/1MiB
[ipc:bench]                         time:   [1.5376 ms 1.5500 ms 1.5626 ms]
[ipc:bench]                         thrpt:  [639.94 MiB/s 645.17 MiB/s 650.36 MiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−2.4073% −1.5439% −0.6900%] (p = 0.01 < 0.05)
[ipc:bench]                         thrpt:  [+0.6948% +1.5681% +2.4667%]
[ipc:bench]                         Change within noise threshold.
[ipc:bench]
[ipc:bench] Benchmarking bidirectional_datagram/server_notify_client_recv
[ipc:bench] Benchmarking bidirectional_datagram/server_notify_client_recv: Warming up for 3.0000 s
[ipc:bench] Benchmarking bidirectional_datagram/server_notify_client_recv: Collecting 20 samples in estimated 30.000 s (5.1M iterations)
[ipc:bench] Benchmarking bidirectional_datagram/server_notify_client_recv: Analyzing
[ipc:bench] bidirectional_datagram/server_notify_client_recv
[ipc:bench]                         time:   [5.9766 µs 6.0550 µs 6.1334 µs]
[ipc:bench]                         thrpt:  [39.805 MiB/s 40.321 MiB/s 40.850 MiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−5.3471% −3.6803% −2.0667%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [+2.1103% +3.8209% +5.6492%]
[ipc:bench]                         Performance has improved.
[ipc:bench] Found 4 outliers among 20 measurements (20.00%)
[ipc:bench]   1 (5.00%) low severe
[ipc:bench]   1 (5.00%) low mild
[ipc:bench]   2 (10.00%) high mild
[ipc:bench]
[ipc:bench] Benchmarking pubsub_fanout/8_publishers_1_event_each
[ipc:bench] Benchmarking pubsub_fanout/8_publishers_1_event_each: Warming up for 3.0000 s
[ipc:bench] Benchmarking pubsub_fanout/8_publishers_1_event_each: Collecting 20 samples in estimated 30.001 s (7.3M iterations)
[ipc:bench] Benchmarking pubsub_fanout/8_publishers_1_event_each: Analyzing
[ipc:bench] pubsub_fanout/8_publishers_1_event_each
[ipc:bench]                         time:   [3.6676 µs 3.6933 µs 3.7264 µs]
[ipc:bench]                         thrpt:  [2.1468 Melem/s 2.1661 Melem/s 2.1813 Melem/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−3.4910% −1.9905% −0.9125%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [+0.9209% +2.0309% +3.6173%]
[ipc:bench]                         Change within noise threshold.
[ipc:bench] Benchmarking pubsub_fanout/1_publisher_100_events
[ipc:bench] Benchmarking pubsub_fanout/1_publisher_100_events: Warming up for 3.0000 s
[ipc:bench] Benchmarking pubsub_fanout/1_publisher_100_events: Collecting 20 samples in estimated 30.000 s (675k iterations)
[ipc:bench] Benchmarking pubsub_fanout/1_publisher_100_events: Analyzing
[ipc:bench] pubsub_fanout/1_publisher_100_events
[ipc:bench]                         time:   [44.429 µs 44.690 µs 44.992 µs]
[ipc:bench]                         thrpt:  [2.2226 Melem/s 2.2376 Melem/s 2.2508 Melem/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−4.3177% −3.7773% −3.1765%] (p = 0.00 < 0.05)
[ipc:bench]                         thrpt:  [+3.2807% +3.9255% +4.5125%]
[ipc:bench]                         Performance has improved.
[ipc:bench]
[ipc:bench] Benchmarking rotation/rotate_idle
[ipc:bench] Benchmarking rotation/rotate_idle: Warming up for 3.0000 s
[ipc:bench] Benchmarking rotation/rotate_idle: Collecting 10 samples in estimated 30.002 s (165k iterations)
[ipc:bench] Benchmarking rotation/rotate_idle: Analyzing
[ipc:bench] rotation/rotate_idle    time:   [178.82 µs 180.77 µs 182.15 µs]
[ipc:bench]                         change: [−4.2835% −3.5865% −2.8702%] (p = 0.00 < 0.05)
[ipc:bench]                         Performance has improved.
[ipc:bench] Benchmarking rotation/rotate_then_request
[ipc:bench] Benchmarking rotation/rotate_then_request: Warming up for 3.0000 s
[ipc:bench] Benchmarking rotation/rotate_then_request: Collecting 10 samples in estimated 30.010 s (166k iterations)
[ipc:bench] Benchmarking rotation/rotate_then_request: Analyzing
[ipc:bench] rotation/rotate_then_request
[ipc:bench]                         time:   [181.35 µs 184.20 µs 187.37 µs]
[ipc:bench]                         change: [−2.0405% −0.1015% +2.1789%] (p = 0.93 > 0.05)
[ipc:bench]                         No change in performance detected.
[ipc:bench] Benchmarking rotation/rotate_then_64KiB_bulk
[ipc:bench] Benchmarking rotation/rotate_then_64KiB_bulk: Warming up for 3.0000 s
[ipc:bench] Benchmarking rotation/rotate_then_64KiB_bulk: Collecting 10 samples in estimated 30.013 s (95k iterations)
[ipc:bench] Benchmarking rotation/rotate_then_64KiB_bulk: Analyzing
[ipc:bench] rotation/rotate_then_64KiB_bulk
[ipc:bench]                         time:   [316.50 µs 320.43 µs 324.00 µs]
[ipc:bench]                         thrpt:  [192.90 MiB/s 195.05 MiB/s 197.47 MiB/s]
[ipc:bench]                  change:
[ipc:bench]                         time:   [−2.4862% −1.3899% −0.1911%] (p = 0.04 < 0.05)
[ipc:bench]                         thrpt:  [+0.1915% +1.4094% +2.5496%]
[ipc:bench]                         Change within noise threshold.
[ipc:bench]
[ipc:bench] Benchmarking rotation_resilience/10_sequential_rotations
[ipc:bench] Benchmarking rotation_resilience/10_sequential_rotations: Warming up for 3.0000 s
[ipc:bench] Benchmarking rotation_resilience/10_sequential_rotations: Collecting 10 samples in estimated 30.045 s (17k iterations)
[ipc:bench] Benchmarking rotation_resilience/10_sequential_rotations: Analyzing
[ipc:bench] rotation_resilience/10_sequential_rotations
[ipc:bench]                         time:   [1.7869 ms 1.7970 ms 1.8091 ms]
[ipc:bench] Benchmarking rotation_resilience/rotate_during_requests
[ipc:bench] Benchmarking rotation_resilience/rotate_during_requests: Warming up for 3.0000 s
[ipc:bench] Benchmarking rotation_resilience/rotate_during_requests: Collecting 10 samples in estimated 30.008 s (17k iterations)
[ipc:bench] Benchmarking rotation_resilience/rotate_during_requests: Analyzing
[ipc:bench] rotation_resilience/rotate_during_requests
[ipc:bench]                         time:   [1.7284 ms 1.7674 ms 1.7877 ms]
[ipc:bench] Benchmarking rotation_resilience/rotate_then_1MiB_bulk
[ipc:bench] Benchmarking rotation_resilience/rotate_then_1MiB_bulk: Warming up for 3.0000 s
[ipc:bench] Benchmarking rotation_resilience/rotate_then_1MiB_bulk: Collecting 10 samples in estimated 30.019 s (18k iterations)
[ipc:bench] Benchmarking rotation_resilience/rotate_then_1MiB_bulk: Analyzing
[ipc:bench] rotation_resilience/rotate_then_1MiB_bulk
[ipc:bench]                         time:   [1.6502 ms 1.6680 ms 1.6834 ms]
[ipc:bench]                         thrpt:  [594.03 MiB/s 599.53 MiB/s 605.99 MiB/s]
[ipc:bench] Benchmarking rotation_resilience/double_rotate_then_bulk
[ipc:bench] Benchmarking rotation_resilience/double_rotate_then_bulk: Warming up for 3.0000 s
[ipc:bench] Benchmarking rotation_resilience/double_rotate_then_bulk: Collecting 10 samples in estimated 30.023 s (17k iterations)
[ipc:bench] Benchmarking rotation_resilience/double_rotate_then_bulk: Analyzing
[ipc:bench] rotation_resilience/double_rotate_then_bulk
[ipc:bench]                         time:   [1.7945 ms 1.8061 ms 1.8190 ms]
[ipc:bench]                         thrpt:  [549.77 MiB/s 553.67 MiB/s 557.25 MiB/s]
[ipc:bench] Found 1 outliers among 10 measurements (10.00%)
[ipc:bench]   1 (10.00%) high severe
[ipc:bench]
[ipc:bench]
[ipc:bench] Workload benchmarks complete.
[ipc:bench] Criterion reports: target/criterion/report/index.html
[ipc:bench] Finished in 5306.13s

usrbinkat@mithril rekindle feat/cli-tui-restructure… 1h28m26s
❯ date
Thu Jun 18 09:43:13 PDT 2026

usrbinkat@mithril rekindle feat/cli-tui-restructure…
❯ ff
usrbinkat@mithril
─────────────────
OS: Pop!_OS 24.04 LTS x86_64
Kernel: Linux 6.18.7-76061807-generic
Uptime: 67 days, 19 hours, 31 mins
DateTime: 2026-06-18 09:45:36 PDT
─────────────────
Host: Precision 7750
Chassis: Notebook
BIOS: 1.29.0 (1.29)
Board: 00K2XV (A00)
─────────────────
CPU: Intel(R) Core(TM) i7-10875H (16) @ 5.10 GHz
CPU Cache: 8x32.00 KiB (D), 8x32.00 KiB (I)
CPU Cache: 8x256.00 KiB (U)
CPU Cache: 16.00 MiB (U)
CPU Usage: 2%
Load Avg: 1.00, 2.06, 3.63
─────────────────
Memory: 27.37 GiB / 125.31 GiB (22%)
Swap: 1.13 GiB / 20.00 GiB (6%)
─────────────────
Disk: 1.38 TiB / 1.82 TiB (76%) - ext4
Physical Disk: 1.86 TiB [SSD, Fixed]
Physical Disk: 1.86 TiB [SSD, Fixed]
GPU: NVIDIA Quadro RTX 5000 Mobile / Max-Q [Discrete]
GPU: Intel UHD Graphics @ 1.20 GHz [Integrated]
─────────────────
Local IP: 192.168.1.66/24 (cc:48:3a:77:f2:20)
DNS: 10.5.0.243 192.168.1.1 192.168.1.1
─────────────────
Init System: systemd 255.4-1ubuntu8.12pop0~1769790828~24.04~d4491c0
Processes: 516
Locale: C.UTF-8
─────────────────
Shell: bash 5.3.3
Terminal: tmux 3.6a
Terminal Size: 145 columns x 39 rows (1015px x 585px)
Editor: nvim 0.11.7
─────────────────
Packages: 2549 (dpkg), 1621 (nix-user), 52 (nix-default), 9 (flatpak-system), 14 (flatpak-user), 9 (snap)
─────────────────
TPM: 2.0
─────────────────
Fastfetch: fastfetch 2.55.1 (x86_64)

usrbinkat@mithril rekindle feat/cli-tui-restructure…
❯

