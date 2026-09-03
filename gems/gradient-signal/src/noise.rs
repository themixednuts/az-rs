//! Gradient noise helpers adapted from O3DE's `GradientSignal` gem.
//!
//! The corresponding O3DE implementation is
//! `Gems/GradientSignal/Code/Source/PerlinImprovedNoise.cpp`, licensed
//! Apache-2.0 OR MIT by Contributors to the Open 3D Engine Project.

const PERMUTATION_TABLE_SIZE: usize = 256;
const DOUBLED_PERMUTATION_TABLE_SIZE: usize = PERMUTATION_TABLE_SIZE * 2;
const MT19937_STATE_SIZE: usize = 624;
const MT19937_PERIOD: usize = 397;
const MT19937_MATRIX_A: u32 = 0x9908_b0df;
const MT19937_UPPER_MASK: u32 = 0x8000_0000;
const MT19937_LOWER_MASK: u32 = 0x7fff_ffff;

/// Improved Perlin noise generator.
///
/// O3DE reference:
/// `Gems/GradientSignal/Code/Include/GradientSignal/PerlinImprovedNoise.h`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PerlinImprovedNoise {
    seed: i32,
    permutation_table: [usize; DOUBLED_PERMUTATION_TABLE_SIZE],
}

impl Default for PerlinImprovedNoise {
    fn default() -> Self {
        Self::new(1)
    }
}

impl PerlinImprovedNoise {
    #[must_use]
    pub fn new(seed: i32) -> Self {
        let seed = seed.max(1);
        let mut noise = Self {
            seed,
            permutation_table: [0; DOUBLED_PERMUTATION_TABLE_SIZE],
        };
        noise.prepare_table();
        noise
    }

    #[must_use]
    pub const fn seed(&self) -> i32 {
        self.seed
    }

    /// Generates normalized octave noise.
    ///
    /// O3DE reference: `Gems/GradientSignal/Code/Source/PerlinImprovedNoise.cpp`.
    // Keep multiplication and addition separate because `mul_add` rounds once
    // and changes deterministic samples produced by this compatibility API.
    #[allow(clippy::suboptimal_flops)]
    #[must_use]
    pub fn generate_octave_noise(
        &self,
        x: f32,
        y: f32,
        z: f32,
        octaves: i32,
        persistence: f32,
        initial_frequency: f32,
    ) -> f32 {
        let mut total = 0.0;
        let mut frequency = initial_frequency;
        let mut amplitude = 1.0;
        let mut max_value = 0.0;

        for _ in 0..octaves.max(0) {
            total += self.generate_noise(x * frequency, y * frequency, z * frequency) * amplitude;
            max_value += amplitude;
            amplitude *= persistence;
            frequency *= 2.0;
        }

        if max_value <= 0.0 {
            return 0.0;
        }
        total / max_value
    }

    /// Generates normalized improved Perlin noise.
    ///
    /// O3DE reference: `Gems/GradientSignal/Code/Source/PerlinImprovedNoise.cpp`.
    #[must_use]
    pub fn generate_noise(&self, x: f32, y: f32, z: f32) -> f32 {
        // Subtracting the `f32` floor directly produces the same fractional
        // value for every coordinate that round-trips through `i32`.
        let floor_x = x.floor();
        let floor_y = y.floor();
        let floor_z = z.floor();
        let xf = x - floor_x;
        let yf = y - floor_y;
        let zf = z - floor_z;
        let xi0 = table_index(floor_x);
        let yi0 = table_index(floor_y);
        let zi0 = table_index(floor_z);
        let xi1 = xi0 + 1;
        let yi1 = yi0 + 1;
        let zi1 = zi0 + 1;
        let fade_x = fade(xf);
        let fade_y = fade(yf);
        let fade_z = fade(zf);
        let perm = &self.permutation_table;

        let aaa = perm[perm[perm[xi0] + yi0] + zi0];
        let aba = perm[perm[perm[xi0] + yi1] + zi0];
        let aab = perm[perm[perm[xi0] + yi0] + zi1];
        let abb = perm[perm[perm[xi0] + yi1] + zi1];
        let baa = perm[perm[perm[xi1] + yi0] + zi0];
        let bba = perm[perm[perm[xi1] + yi1] + zi0];
        let bab = perm[perm[perm[xi1] + yi0] + zi1];
        let bbb = perm[perm[perm[xi1] + yi1] + zi1];

        let x1 = lerp(
            gradient(aaa, xf, yf, zf),
            gradient(baa, xf - 1.0, yf, zf),
            fade_x,
        );
        let x2 = lerp(
            gradient(aba, xf, yf - 1.0, zf),
            gradient(bba, xf - 1.0, yf - 1.0, zf),
            fade_x,
        );
        let y1 = lerp(x1, x2, fade_y);
        let x1 = lerp(
            gradient(aab, xf, yf, zf - 1.0),
            gradient(bab, xf - 1.0, yf, zf - 1.0),
            fade_x,
        );
        let x2 = lerp(
            gradient(abb, xf, yf - 1.0, zf - 1.0),
            gradient(bbb, xf - 1.0, yf - 1.0, zf - 1.0),
            fade_x,
        );
        let y2 = lerp(x1, x2, fade_y);

        lerp(y1, y2, fade_z).midpoint(1.0)
    }

    fn prepare_table(&mut self) {
        let mut table = [0; PERMUTATION_TABLE_SIZE];
        for (index, value) in table.iter_mut().enumerate() {
            *value = index;
        }

        debug_assert!(self.seed >= 1, "`new` clamps the seed to at least 1");
        let mut rng = Mt19937::new(self.seed.unsigned_abs());
        for index in (1..PERMUTATION_TABLE_SIZE).rev() {
            let swap_index = rng.uniform_index(index + 1);
            table.swap(index, swap_index);
        }

        for (index, value) in table.into_iter().enumerate() {
            self.permutation_table[index] = value;
            self.permutation_table[index + PERMUTATION_TABLE_SIZE] = value;
        }
    }
}

/// Wraps a floored lattice coordinate onto the 256-entry permutation table.
///
/// The source writes `(int)floorf(v) & 255`; the mask leaves `0..=255`, so the
/// widening to `usize` is exact and the index is always in bounds.
const fn table_index(floored: f32) -> usize {
    // `f32 -> i32` has no checked form in std (there is no `TryFrom<f32>`), and
    // the saturating cast is defined for coordinates outside `i32`, which the
    // sampled gradient domain never reaches.
    #[allow(clippy::cast_possible_truncation)]
    let cell = floored as i32;
    (cell & 255).unsigned_abs() as usize
}

/// O3DE reference: `Gems/GradientSignal/Code/Source/PerlinImprovedNoise.cpp`.
///
/// O3DE spells 16 cases; three of them repeat an earlier one (`0x0c` is
/// `0x0` with the addends swapped, `0x0d` is `0x9`, `0x0f` is `0x0b`). The
/// patterns are disjoint constants, so merging them is the same function.
const fn gradient(hash: usize, x: f32, y: f32, z: f32) -> f32 {
    match hash & 0x0f {
        0x0 | 0x0c => x + y,
        0x1 => -x + y,
        0x2 => x - y,
        0x3 => -x - y,
        0x4 => x + z,
        0x5 => -x + z,
        0x6 => x - z,
        0x7 => -x - z,
        0x8 => y + z,
        0x9 | 0x0d => -y + z,
        0x0a => y - z,
        0x0b | 0x0f => -y - z,
        0x0e => y - x,
        _ => 0.0,
    }
}

const fn fade(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

const fn lerp(a: f32, b: f32, x: f32) -> f32 {
    a + x * (b - a)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Mt19937 {
    state: [u32; MT19937_STATE_SIZE],
    index: usize,
}

impl Mt19937 {
    fn new(seed: u32) -> Self {
        let mut state = [0; MT19937_STATE_SIZE];
        state[0] = seed;
        for index in 1..MT19937_STATE_SIZE {
            let previous = state[index - 1];
            let counter = u32::try_from(index).expect("the state size is 624");
            state[index] = 1_812_433_253u32
                .wrapping_mul(previous ^ (previous >> 30))
                .wrapping_add(counter);
        }

        Self {
            state,
            index: MT19937_STATE_SIZE,
        }
    }

    fn next_u32(&mut self) -> u32 {
        if self.index >= MT19937_STATE_SIZE {
            self.twist();
        }

        let mut y = self.state[self.index];
        self.index += 1;
        y ^= y >> 11;
        y ^= (y << 7) & 0x9d2c_5680;
        y ^= (y << 15) & 0xefc6_0000;
        y ^ (y >> 18)
    }

    fn uniform_index(&mut self, upper_exclusive: usize) -> usize {
        debug_assert!(upper_exclusive > 0);

        let range = upper_exclusive as u64;
        let limit = (u64::from(u32::MAX) + 1) / range * range;
        loop {
            let value = u64::from(self.next_u32());
            if value < limit {
                return usize::try_from(value % range)
                    .expect("the remainder is below `upper_exclusive`, which is a `usize`");
            }
        }
    }

    fn twist(&mut self) {
        for index in 0..MT19937_STATE_SIZE {
            let x = (self.state[index] & MT19937_UPPER_MASK)
                | (self.state[(index + 1) % MT19937_STATE_SIZE] & MT19937_LOWER_MASK);
            let mut xa = x >> 1;
            if x & 1 != 0 {
                xa ^= MT19937_MATRIX_A;
            }
            self.state[index] = self.state[(index + MT19937_PERIOD) % MT19937_STATE_SIZE] ^ xa;
        }
        self.index = 0;
    }
}
