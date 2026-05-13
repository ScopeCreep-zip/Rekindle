fn main() {
    #[cfg(feature = "aegis")]
    {
        let src = "vendored/libaegis/src";
        let mut build = cc::Build::new();

        // Compile the full libaegis library. aegis_init() in common.c
        // dispatches to ALL variant initializers, so every variant must
        // be compiled even if we only use AEGIS-128L. The variants are
        // small (~200 lines each) and compile in <1s total.
        build
            // Common
            .file(format!("{src}/common/common.c"))
            .file(format!("{src}/common/cpu.c"))
            .file(format!("{src}/common/softaes.c"))
            // AEGIS-128L (our primary cipher)
            .file(format!("{src}/aegis128l/aegis128l.c"))
            .file(format!("{src}/aegis128l/aegis128l_aesni.c"))
            .file(format!("{src}/aegis128l/aegis128l_soft.c"))
            // AEGIS-128X2
            .file(format!("{src}/aegis128x2/aegis128x2.c"))
            .file(format!("{src}/aegis128x2/aegis128x2_aesni.c"))
            .file(format!("{src}/aegis128x2/aegis128x2_avx2.c"))
            .file(format!("{src}/aegis128x2/aegis128x2_soft.c"))
            // AEGIS-128X4
            .file(format!("{src}/aegis128x4/aegis128x4.c"))
            .file(format!("{src}/aegis128x4/aegis128x4_aesni.c"))
            .file(format!("{src}/aegis128x4/aegis128x4_avx2.c"))
            .file(format!("{src}/aegis128x4/aegis128x4_avx512.c"))
            .file(format!("{src}/aegis128x4/aegis128x4_soft.c"))
            // AEGIS-256
            .file(format!("{src}/aegis256/aegis256.c"))
            .file(format!("{src}/aegis256/aegis256_aesni.c"))
            .file(format!("{src}/aegis256/aegis256_soft.c"))
            // AEGIS-256X2
            .file(format!("{src}/aegis256x2/aegis256x2.c"))
            .file(format!("{src}/aegis256x2/aegis256x2_aesni.c"))
            .file(format!("{src}/aegis256x2/aegis256x2_avx2.c"))
            .file(format!("{src}/aegis256x2/aegis256x2_soft.c"))
            // AEGIS-256X4
            .file(format!("{src}/aegis256x4/aegis256x4.c"))
            .file(format!("{src}/aegis256x4/aegis256x4_aesni.c"))
            .file(format!("{src}/aegis256x4/aegis256x4_avx2.c"))
            .file(format!("{src}/aegis256x4/aegis256x4_avx512.c"))
            .file(format!("{src}/aegis256x4/aegis256x4_soft.c"))
            // Include paths
            .include(format!("{src}/include"))
            .include(format!("{src}/common"))
            .flag_if_supported("-maes")
            .flag_if_supported("-msse4.1")
            .flag_if_supported("-mavx2")
            .flag_if_supported("-O3")
            .warnings(false)
            .compile("aegis");
    }
}
