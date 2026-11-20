//! The display encoding of an image: what a screen, a 2D canvas or an 8-bit file takes.
//!
//! An image is linear light, BT.709 primaries, alpha premultiplied, four 32-bit floats a
//! pixel ([`crate::ColorEncoding`]); a display takes 8-bit sRGB with straight alpha. The
//! encoding (`spec/visual-design.md`, section 9, "Display encoding") takes each color over alpha as
//! the largest 32-bit float at or below the real quotient (never a rounded division),
//! encodes it with the sRGB transfer function of IEC 61966-2-1 and rounds it to the nearest
//! of the 256 codes, a tie to the higher code; alpha is rounded alike, linearly. It is exact
//! and the same on every target, a GPU's included: each code is found by comparing with the
//! thresholds below, the smallest 32-bit float that reaches each code, computed once in
//! exact rational arithmetic and proven entry by entry by the tests; no power function,
//! whose rounding differs between targets, is ever evaluated, and no division, which a GPU
//! need not round correctly: the thresholds being floats, a threshold `t` is at or below
//! that float exactly when `t × alpha` is at most the color, a product two 32-bit floats
//! make exactly in 64 bits.

use alloc::collections::TryReserveError;
use alloc::vec::Vec;

use crate::{Holding, Image, ImageSize, VisualError};

/// For each code from 1 to 255, the smallest 32-bit float whose sRGB encoding rounds to it
/// or above: a linear value `v` encodes to the count of thresholds at or below it.
///
/// Entry `k` is the smallest float `t` with `OETF(t) ≥ (k + ½) / 255`, where `OETF(t)` is
/// `12.92 t` up to 0.003 130 8 and `1.055 t^(1/2.4) − 0.055` above (IEC 61966-2-1).
pub const SRGB8_THRESHOLDS: [f32; 255] = [
    f32::from_bits(0x391F_22B4),
    f32::from_bits(0x39EE_B40E),
    f32::from_bits(0x3A46_EB61),
    f32::from_bits(0x3A8B_3E5E),
    f32::from_bits(0x3AB3_070B),
    f32::from_bits(0x3ADA_CFB8),
    f32::from_bits(0x3B01_4C33),
    f32::from_bits(0x3B15_3089),
    f32::from_bits(0x3B29_14DF),
    f32::from_bits(0x3B3C_F936),
    f32::from_bits(0x3B50_F2D1),
    f32::from_bits(0x3B65_FB9B),
    f32::from_bits(0x3B7C_3403),
    f32::from_bits(0x3B89_D060),
    f32::from_bits(0x3B96_2333),
    f32::from_bits(0x3BA3_14BD),
    f32::from_bits(0x3BB0_A731),
    f32::from_bits(0x3BBE_DCB7),
    f32::from_bits(0x3BCD_B76D),
    f32::from_bits(0x3BDD_3967),
    f32::from_bits(0x3BED_64AF),
    f32::from_bits(0x3BFE_3B46),
    f32::from_bits(0x3C07_DF91),
    f32::from_bits(0x3C10_F91B),
    f32::from_bits(0x3C1A_6B32),
    f32::from_bits(0x3C24_36C8),
    f32::from_bits(0x3C2E_5CC7),
    f32::from_bits(0x3C38_DE1A),
    f32::from_bits(0x3C43_BBA4),
    f32::from_bits(0x3C4E_F648),
    f32::from_bits(0x3C5A_8EE4),
    f32::from_bits(0x3C66_8654),
    f32::from_bits(0x3C72_DD71),
    f32::from_bits(0x3C7F_950F),
    f32::from_bits(0x3C86_5702),
    f32::from_bits(0x3C8D_148F),
    f32::from_bits(0x3C94_0396),
    f32::from_bits(0x3C9B_247C),
    f32::from_bits(0x3CA2_77A6),
    f32::from_bits(0x3CA9_FD78),
    f32::from_bits(0x3CB1_B653),
    f32::from_bits(0x3CB9_A298),
    f32::from_bits(0x3CC1_C2A9),
    f32::from_bits(0x3CCA_16E3),
    f32::from_bits(0x3CD2_9FA4),
    f32::from_bits(0x3CDB_5D4B),
    f32::from_bits(0x3CE4_5032),
    f32::from_bits(0x3CED_78B5),
    f32::from_bits(0x3CF6_D72E),
    f32::from_bits(0x3D00_35FC),
    f32::from_bits(0x3D05_1BB4),
    f32::from_bits(0x3D0A_1CEC),
    f32::from_bits(0x3D0F_39D0),
    f32::from_bits(0x3D14_728A),
    f32::from_bits(0x3D19_C745),
    f32::from_bits(0x3D1F_382C),
    f32::from_bits(0x3D24_C567),
    f32::from_bits(0x3D2A_6F22),
    f32::from_bits(0x3D30_3584),
    f32::from_bits(0x3D36_18B7),
    f32::from_bits(0x3D3C_18E4),
    f32::from_bits(0x3D42_3632),
    f32::from_bits(0x3D48_70CA),
    f32::from_bits(0x3D4E_C8D2),
    f32::from_bits(0x3D55_3E73),
    f32::from_bits(0x3D5B_D1D3),
    f32::from_bits(0x3D62_8318),
    f32::from_bits(0x3D69_526A),
    f32::from_bits(0x3D70_3FEE),
    f32::from_bits(0x3D77_4BCA),
    f32::from_bits(0x3D7E_7624),
    f32::from_bits(0x3D82_DF90),
    f32::from_bits(0x3D86_9372),
    f32::from_bits(0x3D8A_56CB),
    f32::from_bits(0x3D8E_29AB),
    f32::from_bits(0x3D92_0C27),
    f32::from_bits(0x3D95_FE4F),
    f32::from_bits(0x3D9A_0035),
    f32::from_bits(0x3D9E_11EC),
    f32::from_bits(0x3DA2_3384),
    f32::from_bits(0x3DA6_6510),
    f32::from_bits(0x3DAA_A6A0),
    f32::from_bits(0x3DAE_F847),
    f32::from_bits(0x3DB3_5A15),
    f32::from_bits(0x3DB7_CC1B),
    f32::from_bits(0x3DBC_4E6B),
    f32::from_bits(0x3DC0_E114),
    f32::from_bits(0x3DC5_8429),
    f32::from_bits(0x3DCA_37B9),
    f32::from_bits(0x3DCE_FBD6),
    f32::from_bits(0x3DD3_D08F),
    f32::from_bits(0x3DD8_B5F5),
    f32::from_bits(0x3DDD_AC19),
    f32::from_bits(0x3DE2_B30A),
    f32::from_bits(0x3DE7_CAD9),
    f32::from_bits(0x3DEC_F395),
    f32::from_bits(0x3DF2_2D50),
    f32::from_bits(0x3DF7_7817),
    f32::from_bits(0x3DFC_D3FC),
    f32::from_bits(0x3E01_2087),
    f32::from_bits(0x3E03_DFAE),
    f32::from_bits(0x3E06_A77B),
    f32::from_bits(0x3E09_77F6),
    f32::from_bits(0x3E0C_5126),
    f32::from_bits(0x3E0F_3314),
    f32::from_bits(0x3E12_1DC5),
    f32::from_bits(0x3E15_1143),
    f32::from_bits(0x3E18_0D95),
    f32::from_bits(0x3E1B_12C2),
    f32::from_bits(0x3E1E_20D1),
    f32::from_bits(0x3E21_37CB),
    f32::from_bits(0x3E24_57B6),
    f32::from_bits(0x3E27_8099),
    f32::from_bits(0x3E2A_B27D),
    f32::from_bits(0x3E2D_ED68),
    f32::from_bits(0x3E31_3161),
    f32::from_bits(0x3E34_7E70),
    f32::from_bits(0x3E37_D49C),
    f32::from_bits(0x3E3B_33EC),
    f32::from_bits(0x3E3E_9C67),
    f32::from_bits(0x3E42_0E15),
    f32::from_bits(0x3E45_88FB),
    f32::from_bits(0x3E49_0D22),
    f32::from_bits(0x3E4C_9A90),
    f32::from_bits(0x3E50_314C),
    f32::from_bits(0x3E53_D15D),
    f32::from_bits(0x3E57_7ACA),
    f32::from_bits(0x3E5B_2D9A),
    f32::from_bits(0x3E5E_E9D4),
    f32::from_bits(0x3E62_AF7E),
    f32::from_bits(0x3E66_7E9F),
    f32::from_bits(0x3E6A_573E),
    f32::from_bits(0x3E6E_3962),
    f32::from_bits(0x3E72_2511),
    f32::from_bits(0x3E76_1A52),
    f32::from_bits(0x3E7A_192C),
    f32::from_bits(0x3E7E_21A5),
    f32::from_bits(0x3E81_19E2),
    f32::from_bits(0x3E83_27C7),
    f32::from_bits(0x3E85_3A86),
    f32::from_bits(0x3E87_5222),
    f32::from_bits(0x3E89_6E9D),
    f32::from_bits(0x3E8B_8FFC),
    f32::from_bits(0x3E8D_B641),
    f32::from_bits(0x3E8F_E170),
    f32::from_bits(0x3E92_118B),
    f32::from_bits(0x3E94_4696),
    f32::from_bits(0x3E96_8095),
    f32::from_bits(0x3E98_BF89),
    f32::from_bits(0x3E9B_0377),
    f32::from_bits(0x3E9D_4C62),
    f32::from_bits(0x3E9F_9A4C),
    f32::from_bits(0x3EA1_ED38),
    f32::from_bits(0x3EA4_452B),
    f32::from_bits(0x3EA6_A226),
    f32::from_bits(0x3EA9_042E),
    f32::from_bits(0x3EAB_6B44),
    f32::from_bits(0x3EAD_D76D),
    f32::from_bits(0x3EB0_48AA),
    f32::from_bits(0x3EB2_BF00),
    f32::from_bits(0x3EB5_3A71),
    f32::from_bits(0x3EB7_BB00),
    f32::from_bits(0x3EBA_40B1),
    f32::from_bits(0x3EBC_CB85),
    f32::from_bits(0x3EBF_5B81),
    f32::from_bits(0x3EC1_F0A7),
    f32::from_bits(0x3EC4_8AF9),
    f32::from_bits(0x3EC7_2A7C),
    f32::from_bits(0x3EC9_CF32),
    f32::from_bits(0x3ECC_791E),
    f32::from_bits(0x3ECF_2842),
    f32::from_bits(0x3ED1_DCA2),
    f32::from_bits(0x3ED4_9641),
    f32::from_bits(0x3ED7_5521),
    f32::from_bits(0x3EDA_1946),
    f32::from_bits(0x3EDC_E2B2),
    f32::from_bits(0x3EDF_B168),
    f32::from_bits(0x3EE2_856A),
    f32::from_bits(0x3EE5_5EBD),
    f32::from_bits(0x3EE8_3D63),
    f32::from_bits(0x3EEB_215D),
    f32::from_bits(0x3EEE_0AB1),
    f32::from_bits(0x3EF0_F95F),
    f32::from_bits(0x3EF3_ED6B),
    f32::from_bits(0x3EF6_E6D8),
    f32::from_bits(0x3EF9_E5A8),
    f32::from_bits(0x3EFC_E9DE),
    f32::from_bits(0x3EFF_F37E),
    f32::from_bits(0x3F01_8145),
    f32::from_bits(0x3F03_0B82),
    f32::from_bits(0x3F04_9877),
    f32::from_bits(0x3F06_2827),
    f32::from_bits(0x3F07_BA92),
    f32::from_bits(0x3F09_4FB9),
    f32::from_bits(0x3F0A_E79F),
    f32::from_bits(0x3F0C_8244),
    f32::from_bits(0x3F0E_1FAA),
    f32::from_bits(0x3F0F_BFD2),
    f32::from_bits(0x3F11_62BE),
    f32::from_bits(0x3F13_086E),
    f32::from_bits(0x3F14_B0E4),
    f32::from_bits(0x3F16_5C22),
    f32::from_bits(0x3F18_0A29),
    f32::from_bits(0x3F19_BAFA),
    f32::from_bits(0x3F1B_6E96),
    f32::from_bits(0x3F1D_24FF),
    f32::from_bits(0x3F1E_DE36),
    f32::from_bits(0x3F20_9A3C),
    f32::from_bits(0x3F22_5913),
    f32::from_bits(0x3F24_1ABC),
    f32::from_bits(0x3F25_DF38),
    f32::from_bits(0x3F27_A689),
    f32::from_bits(0x3F29_70AF),
    f32::from_bits(0x3F2B_3DAD),
    f32::from_bits(0x3F2D_0D83),
    f32::from_bits(0x3F2E_E032),
    f32::from_bits(0x3F30_B5BD),
    f32::from_bits(0x3F32_8E24),
    f32::from_bits(0x3F34_6968),
    f32::from_bits(0x3F36_478B),
    f32::from_bits(0x3F38_288F),
    f32::from_bits(0x3F3A_0C73),
    f32::from_bits(0x3F3B_F33A),
    f32::from_bits(0x3F3D_DCE5),
    f32::from_bits(0x3F3F_C975),
    f32::from_bits(0x3F41_B8EB),
    f32::from_bits(0x3F43_AB48),
    f32::from_bits(0x3F45_A08F),
    f32::from_bits(0x3F47_98BF),
    f32::from_bits(0x3F49_93DB),
    f32::from_bits(0x3F4B_91E3),
    f32::from_bits(0x3F4D_92D8),
    f32::from_bits(0x3F4F_96BD),
    f32::from_bits(0x3F51_9D92),
    f32::from_bits(0x3F53_A758),
    f32::from_bits(0x3F55_B411),
    f32::from_bits(0x3F57_C3BE),
    f32::from_bits(0x3F59_D65F),
    f32::from_bits(0x3F5B_EBF7),
    f32::from_bits(0x3F5E_0486),
    f32::from_bits(0x3F60_200E),
    f32::from_bits(0x3F62_3E90),
    f32::from_bits(0x3F64_600C),
    f32::from_bits(0x3F66_8485),
    f32::from_bits(0x3F68_ABFB),
    f32::from_bits(0x3F6A_D670),
    f32::from_bits(0x3F6D_03E5),
    f32::from_bits(0x3F6F_345A),
    f32::from_bits(0x3F71_67D2),
    f32::from_bits(0x3F73_9E4D),
    f32::from_bits(0x3F75_D7CC),
    f32::from_bits(0x3F78_1451),
    f32::from_bits(0x3F7A_53DD),
    f32::from_bits(0x3F7C_9671),
    f32::from_bits(0x3F7E_DC0E),
];

/// For each code from 1 to 255, the smallest 32-bit float at or above `(k + ½) / 255`: the
/// linear rounding of alpha to the nearest code, a tie to the higher.
pub const ALPHA8_THRESHOLDS: [f32; 255] = [
    f32::from_bits(0x3B00_8081),
    f32::from_bits(0x3BC0_C0C1),
    f32::from_bits(0x3C20_A0A1),
    f32::from_bits(0x3C60_E0E1),
    f32::from_bits(0x3C90_9091),
    f32::from_bits(0x3CB0_B0B1),
    f32::from_bits(0x3CD0_D0D1),
    f32::from_bits(0x3CF0_F0F1),
    f32::from_bits(0x3D08_8889),
    f32::from_bits(0x3D18_9899),
    f32::from_bits(0x3D28_A8A9),
    f32::from_bits(0x3D38_B8B9),
    f32::from_bits(0x3D48_C8C9),
    f32::from_bits(0x3D58_D8D9),
    f32::from_bits(0x3D68_E8E9),
    f32::from_bits(0x3D78_F8F9),
    f32::from_bits(0x3D84_8485),
    f32::from_bits(0x3D8C_8C8D),
    f32::from_bits(0x3D94_9495),
    f32::from_bits(0x3D9C_9C9D),
    f32::from_bits(0x3DA4_A4A5),
    f32::from_bits(0x3DAC_ACAD),
    f32::from_bits(0x3DB4_B4B5),
    f32::from_bits(0x3DBC_BCBD),
    f32::from_bits(0x3DC4_C4C5),
    f32::from_bits(0x3DCC_CCCD),
    f32::from_bits(0x3DD4_D4D5),
    f32::from_bits(0x3DDC_DCDD),
    f32::from_bits(0x3DE4_E4E5),
    f32::from_bits(0x3DEC_ECED),
    f32::from_bits(0x3DF4_F4F5),
    f32::from_bits(0x3DFC_FCFD),
    f32::from_bits(0x3E02_8283),
    f32::from_bits(0x3E06_8687),
    f32::from_bits(0x3E0A_8A8B),
    f32::from_bits(0x3E0E_8E8F),
    f32::from_bits(0x3E12_9293),
    f32::from_bits(0x3E16_9697),
    f32::from_bits(0x3E1A_9A9B),
    f32::from_bits(0x3E1E_9E9F),
    f32::from_bits(0x3E22_A2A3),
    f32::from_bits(0x3E26_A6A7),
    f32::from_bits(0x3E2A_AAAB),
    f32::from_bits(0x3E2E_AEAF),
    f32::from_bits(0x3E32_B2B3),
    f32::from_bits(0x3E36_B6B7),
    f32::from_bits(0x3E3A_BABB),
    f32::from_bits(0x3E3E_BEBF),
    f32::from_bits(0x3E42_C2C3),
    f32::from_bits(0x3E46_C6C7),
    f32::from_bits(0x3E4A_CACB),
    f32::from_bits(0x3E4E_CECF),
    f32::from_bits(0x3E52_D2D3),
    f32::from_bits(0x3E56_D6D7),
    f32::from_bits(0x3E5A_DADB),
    f32::from_bits(0x3E5E_DEDF),
    f32::from_bits(0x3E62_E2E3),
    f32::from_bits(0x3E66_E6E7),
    f32::from_bits(0x3E6A_EAEB),
    f32::from_bits(0x3E6E_EEEF),
    f32::from_bits(0x3E72_F2F3),
    f32::from_bits(0x3E76_F6F7),
    f32::from_bits(0x3E7A_FAFB),
    f32::from_bits(0x3E7E_FEFF),
    f32::from_bits(0x3E81_8182),
    f32::from_bits(0x3E83_8384),
    f32::from_bits(0x3E85_8586),
    f32::from_bits(0x3E87_8788),
    f32::from_bits(0x3E89_898A),
    f32::from_bits(0x3E8B_8B8C),
    f32::from_bits(0x3E8D_8D8E),
    f32::from_bits(0x3E8F_8F90),
    f32::from_bits(0x3E91_9192),
    f32::from_bits(0x3E93_9394),
    f32::from_bits(0x3E95_9596),
    f32::from_bits(0x3E97_9798),
    f32::from_bits(0x3E99_999A),
    f32::from_bits(0x3E9B_9B9C),
    f32::from_bits(0x3E9D_9D9E),
    f32::from_bits(0x3E9F_9FA0),
    f32::from_bits(0x3EA1_A1A2),
    f32::from_bits(0x3EA3_A3A4),
    f32::from_bits(0x3EA5_A5A6),
    f32::from_bits(0x3EA7_A7A8),
    f32::from_bits(0x3EA9_A9AA),
    f32::from_bits(0x3EAB_ABAC),
    f32::from_bits(0x3EAD_ADAE),
    f32::from_bits(0x3EAF_AFB0),
    f32::from_bits(0x3EB1_B1B2),
    f32::from_bits(0x3EB3_B3B4),
    f32::from_bits(0x3EB5_B5B6),
    f32::from_bits(0x3EB7_B7B8),
    f32::from_bits(0x3EB9_B9BA),
    f32::from_bits(0x3EBB_BBBC),
    f32::from_bits(0x3EBD_BDBE),
    f32::from_bits(0x3EBF_BFC0),
    f32::from_bits(0x3EC1_C1C2),
    f32::from_bits(0x3EC3_C3C4),
    f32::from_bits(0x3EC5_C5C6),
    f32::from_bits(0x3EC7_C7C8),
    f32::from_bits(0x3EC9_C9CA),
    f32::from_bits(0x3ECB_CBCC),
    f32::from_bits(0x3ECD_CDCE),
    f32::from_bits(0x3ECF_CFD0),
    f32::from_bits(0x3ED1_D1D2),
    f32::from_bits(0x3ED3_D3D4),
    f32::from_bits(0x3ED5_D5D6),
    f32::from_bits(0x3ED7_D7D8),
    f32::from_bits(0x3ED9_D9DA),
    f32::from_bits(0x3EDB_DBDC),
    f32::from_bits(0x3EDD_DDDE),
    f32::from_bits(0x3EDF_DFE0),
    f32::from_bits(0x3EE1_E1E2),
    f32::from_bits(0x3EE3_E3E4),
    f32::from_bits(0x3EE5_E5E6),
    f32::from_bits(0x3EE7_E7E8),
    f32::from_bits(0x3EE9_E9EA),
    f32::from_bits(0x3EEB_EBEC),
    f32::from_bits(0x3EED_EDEE),
    f32::from_bits(0x3EEF_EFF0),
    f32::from_bits(0x3EF1_F1F2),
    f32::from_bits(0x3EF3_F3F4),
    f32::from_bits(0x3EF5_F5F6),
    f32::from_bits(0x3EF7_F7F8),
    f32::from_bits(0x3EF9_F9FA),
    f32::from_bits(0x3EFB_FBFC),
    f32::from_bits(0x3EFD_FDFE),
    f32::from_bits(0x3F00_0000),
    f32::from_bits(0x3F01_0102),
    f32::from_bits(0x3F02_0203),
    f32::from_bits(0x3F03_0304),
    f32::from_bits(0x3F04_0405),
    f32::from_bits(0x3F05_0506),
    f32::from_bits(0x3F06_0607),
    f32::from_bits(0x3F07_0708),
    f32::from_bits(0x3F08_0809),
    f32::from_bits(0x3F09_090A),
    f32::from_bits(0x3F0A_0A0B),
    f32::from_bits(0x3F0B_0B0C),
    f32::from_bits(0x3F0C_0C0D),
    f32::from_bits(0x3F0D_0D0E),
    f32::from_bits(0x3F0E_0E0F),
    f32::from_bits(0x3F0F_0F10),
    f32::from_bits(0x3F10_1011),
    f32::from_bits(0x3F11_1112),
    f32::from_bits(0x3F12_1213),
    f32::from_bits(0x3F13_1314),
    f32::from_bits(0x3F14_1415),
    f32::from_bits(0x3F15_1516),
    f32::from_bits(0x3F16_1617),
    f32::from_bits(0x3F17_1718),
    f32::from_bits(0x3F18_1819),
    f32::from_bits(0x3F19_191A),
    f32::from_bits(0x3F1A_1A1B),
    f32::from_bits(0x3F1B_1B1C),
    f32::from_bits(0x3F1C_1C1D),
    f32::from_bits(0x3F1D_1D1E),
    f32::from_bits(0x3F1E_1E1F),
    f32::from_bits(0x3F1F_1F20),
    f32::from_bits(0x3F20_2021),
    f32::from_bits(0x3F21_2122),
    f32::from_bits(0x3F22_2223),
    f32::from_bits(0x3F23_2324),
    f32::from_bits(0x3F24_2425),
    f32::from_bits(0x3F25_2526),
    f32::from_bits(0x3F26_2627),
    f32::from_bits(0x3F27_2728),
    f32::from_bits(0x3F28_2829),
    f32::from_bits(0x3F29_292A),
    f32::from_bits(0x3F2A_2A2B),
    f32::from_bits(0x3F2B_2B2C),
    f32::from_bits(0x3F2C_2C2D),
    f32::from_bits(0x3F2D_2D2E),
    f32::from_bits(0x3F2E_2E2F),
    f32::from_bits(0x3F2F_2F30),
    f32::from_bits(0x3F30_3031),
    f32::from_bits(0x3F31_3132),
    f32::from_bits(0x3F32_3233),
    f32::from_bits(0x3F33_3334),
    f32::from_bits(0x3F34_3435),
    f32::from_bits(0x3F35_3536),
    f32::from_bits(0x3F36_3637),
    f32::from_bits(0x3F37_3738),
    f32::from_bits(0x3F38_3839),
    f32::from_bits(0x3F39_393A),
    f32::from_bits(0x3F3A_3A3B),
    f32::from_bits(0x3F3B_3B3C),
    f32::from_bits(0x3F3C_3C3D),
    f32::from_bits(0x3F3D_3D3E),
    f32::from_bits(0x3F3E_3E3F),
    f32::from_bits(0x3F3F_3F40),
    f32::from_bits(0x3F40_4041),
    f32::from_bits(0x3F41_4142),
    f32::from_bits(0x3F42_4243),
    f32::from_bits(0x3F43_4344),
    f32::from_bits(0x3F44_4445),
    f32::from_bits(0x3F45_4546),
    f32::from_bits(0x3F46_4647),
    f32::from_bits(0x3F47_4748),
    f32::from_bits(0x3F48_4849),
    f32::from_bits(0x3F49_494A),
    f32::from_bits(0x3F4A_4A4B),
    f32::from_bits(0x3F4B_4B4C),
    f32::from_bits(0x3F4C_4C4D),
    f32::from_bits(0x3F4D_4D4E),
    f32::from_bits(0x3F4E_4E4F),
    f32::from_bits(0x3F4F_4F50),
    f32::from_bits(0x3F50_5051),
    f32::from_bits(0x3F51_5152),
    f32::from_bits(0x3F52_5253),
    f32::from_bits(0x3F53_5354),
    f32::from_bits(0x3F54_5455),
    f32::from_bits(0x3F55_5556),
    f32::from_bits(0x3F56_5657),
    f32::from_bits(0x3F57_5758),
    f32::from_bits(0x3F58_5859),
    f32::from_bits(0x3F59_595A),
    f32::from_bits(0x3F5A_5A5B),
    f32::from_bits(0x3F5B_5B5C),
    f32::from_bits(0x3F5C_5C5D),
    f32::from_bits(0x3F5D_5D5E),
    f32::from_bits(0x3F5E_5E5F),
    f32::from_bits(0x3F5F_5F60),
    f32::from_bits(0x3F60_6061),
    f32::from_bits(0x3F61_6162),
    f32::from_bits(0x3F62_6263),
    f32::from_bits(0x3F63_6364),
    f32::from_bits(0x3F64_6465),
    f32::from_bits(0x3F65_6566),
    f32::from_bits(0x3F66_6667),
    f32::from_bits(0x3F67_6768),
    f32::from_bits(0x3F68_6869),
    f32::from_bits(0x3F69_696A),
    f32::from_bits(0x3F6A_6A6B),
    f32::from_bits(0x3F6B_6B6C),
    f32::from_bits(0x3F6C_6C6D),
    f32::from_bits(0x3F6D_6D6E),
    f32::from_bits(0x3F6E_6E6F),
    f32::from_bits(0x3F6F_6F70),
    f32::from_bits(0x3F70_7071),
    f32::from_bits(0x3F71_7172),
    f32::from_bits(0x3F72_7273),
    f32::from_bits(0x3F73_7374),
    f32::from_bits(0x3F74_7475),
    f32::from_bits(0x3F75_7576),
    f32::from_bits(0x3F76_7677),
    f32::from_bits(0x3F77_7778),
    f32::from_bits(0x3F78_7879),
    f32::from_bits(0x3F79_797A),
    f32::from_bits(0x3F7A_7A7B),
    f32::from_bits(0x3F7B_7B7C),
    f32::from_bits(0x3F7C_7C7D),
    f32::from_bits(0x3F7D_7D7E),
    f32::from_bits(0x3F7E_7E7F),
    f32::from_bits(0x3F7F_7F80),
];

/// The WGSL source of the display encoding on a GPU (`display.wgsl`).
///
/// Given with the library so that every GPU interface (wgpu natively, WebGPU in browsers)
/// runs this one: the bytes [`pixel8`] gives, exactly, in 32-bit integers. It reads
/// [`DISPLAY_THRESHOLD_BITS`].
pub const DISPLAY_WGSL: &str = include_str!("display.wgsl");

/// The thresholds as the display shader reads them: the 255 of [`SRGB8_THRESHOLDS`] then the
/// 255 of [`ALPHA8_THRESHOLDS`], each as its bits.
#[expect(
    clippy::indexing_slicing,
    reason = "evaluated at compile time: an index out of bounds fails the build, never a run"
)]
pub const DISPLAY_THRESHOLD_BITS: [u32; 510] = {
    let mut bits = [0_u32; 510];
    let mut at = 0;
    while at < 255 {
        bits[at] = SRGB8_THRESHOLDS[at].to_bits();
        bits[at + 255] = ALPHA8_THRESHOLDS[at].to_bits();
        // A step no mutation tool rewrites into one that never ends: a constant that never
        // ends would hang the compiler, not fail a test.
        at = at.wrapping_add(1);
    }
    bits
};

/// The bits of the smallest float a bucket of [`SRGB8_STARTS`] and [`ALPHA8_STARTS`] holds,
/// 2⁻¹³: below every threshold.
const LOW_BITS: u32 = 0x3900_0000;

/// The bits of 1, from which every value encodes to 255: above every threshold.
const HIGH_BITS: u32 = 0x3F80_0000;

/// The bits of positive infinity: above them, NaNs, then every float of negative sign.
const INFINITY_BITS: u32 = 0x7F80_0000;

/// The low bits a bucket spans: a bucket holds the floats of the same 16 high bits.
const BUCKET_SHIFT: u32 = 16;

/// The buckets from [`LOW_BITS`] to [`HIGH_BITS`].
const BUCKETS: usize = 0x680;

/// For each bucket of floats (the same 16 high bits, from [`LOW_BITS`] to [`HIGH_BITS`]),
/// how many of `thresholds` are below its smallest float: the code every float of the bucket
/// reaches at least, before the threshold inside it, if any (one at most: consecutive
/// thresholds are further apart than a bucket's 65,536 floats, which a test checks).
#[expect(
    clippy::indexing_slicing,
    reason = "evaluated at compile time: an index out of bounds fails the build, never a run"
)]
const fn starts(thresholds: &[f32; 255]) -> [u8; BUCKETS] {
    let mut starts = [0_u8; BUCKETS];
    // The bucket, as an index and as the high bits of its smallest float, walked together
    // (no conversion between them in a constant); every step stays far below either's
    // largest value, so no wrap happens.
    let (mut bucket, mut high) = (0_usize, LOW_BITS >> BUCKET_SHIFT);
    // The thresholds below the bucket, as an index and as a code.
    let (mut below, mut code) = (0_usize, 0_u8);
    while bucket < BUCKETS {
        while below < 255 && thresholds[below].to_bits() < high << BUCKET_SHIFT {
            below = below.wrapping_add(1);
            code = code.wrapping_add(1);
        }
        starts[bucket] = code;
        bucket = bucket.wrapping_add(1);
        high = high.wrapping_add(1);
    }
    starts
}

/// The thresholds below the bucket of the float of `bits`, from [`LOW_BITS`] to
/// [`HIGH_BITS`] (0 for any other).
fn bucket_start(starts: &[u8; BUCKETS], bits: u32) -> usize {
    let bucket = (bits >> BUCKET_SHIFT).saturating_sub(LOW_BITS >> BUCKET_SHIFT);
    usize::try_from(bucket)
        .ok()
        .and_then(|at| starts.get(at))
        .map_or(0, |&start| usize::from(start))
}

/// The smallest float of each bucket of the codes' tables, in order: 1,664 values, from
/// 2⁻¹³ to the bucket below 1 (the buckets of `spec/visual-design.md`, section 9).
///
/// A bucket is a run of floats sharing their high bits, from which [`srgb8`] and
/// [`alpha8`] start counting thresholds: the layout of an internal accelerator, which
/// changes only their speed, never a code, and is not a stable contract. One value reaching
/// each table entry, which a corpus of the display encoding holds (`visual-cli`'s), read
/// from here, never from a copy of the layout: changing the layout changes these values,
/// and so that corpus, which must then be blessed again.
pub fn display_bucket_floors() -> impl Iterator<Item = f32> {
    ((LOW_BITS >> BUCKET_SHIFT)..(HIGH_BITS >> BUCKET_SHIFT))
        .map(|high| f32::from_bits(high << BUCKET_SHIFT))
}

/// [`starts`] of [`SRGB8_THRESHOLDS`].
const SRGB8_STARTS: [u8; BUCKETS] = starts(&SRGB8_THRESHOLDS);

/// [`starts`] of [`ALPHA8_THRESHOLDS`].
const ALPHA8_STARTS: [u8; BUCKETS] = starts(&ALPHA8_THRESHOLDS);

/// The code of `value` given the `thresholds` of codes 1 to 255: how many are at or below it
/// (0 below the first, and for every value that is not a number).
///
/// For a float of positive sign that is not a number, the order of the bits is its order:
/// its bucket gives the thresholds below it ([`starts`]), the one inside the bucket, if any,
/// is then compared. The count is exact whatever the bucket holds; the buckets only make it
/// fast (one comparison, where a binary search took eight).
fn code(thresholds: &[f32; 255], starts: &[u8; BUCKETS], value: f32) -> u8 {
    let bits = value.to_bits();
    if !(LOW_BITS..=INFINITY_BITS).contains(&bits) {
        // Not a number, of negative sign (-0 included), or below every threshold.
        return 0;
    }
    if bits >= HIGH_BITS {
        return u8::MAX;
    }
    let start = bucket_start(starts, bits);
    let inside = thresholds.get(start..).map_or(0, |rest| {
        rest.iter()
            .take_while(|&&threshold| threshold <= value)
            .count()
    });
    u8::try_from(start.saturating_add(inside)).unwrap_or(u8::MAX)
}

/// The 8-bit sRGB code of the linear-light value `value`: 0 at or below 0, 255 from the
/// last threshold on (values above 1 included), exact.
#[must_use]
pub fn srgb8(value: f32) -> u8 {
    code(&SRGB8_THRESHOLDS, &SRGB8_STARTS, value)
}

/// The 8-bit code of the alpha `value`: the nearest of `k / 255`, a tie to the higher;
/// 0 at or below 0, 255 at or above 1, exact.
#[must_use]
pub fn alpha8(value: f32) -> u8 {
    code(&ALPHA8_THRESHOLDS, &ALPHA8_STARTS, value)
}

/// The 8-bit sRGB code of `color / alpha`, the straight value of a premultiplied color,
/// for an `alpha` whose code is above 0 ([`alpha8`]), as [`pixel8`] calls it: over an alpha
/// at or below 0, or not a number, the quotient has no code, and the pixel is transparent
/// black instead.
///
/// The straight value is the largest 32-bit float at or below the real quotient; the
/// quotient itself is never encoded (between a threshold and the float below it, it would
/// take one code more). Its code is counted exactly: how many thresholds `t` have
/// `t × alpha` at most `color` (a float `t` is at or below the quotient exactly then), each
/// product exact in 64 bits (two 24-bit significands make at most 48 bits, and no product
/// of 32-bit floats leaves the normal range of a 64-bit one). No division is rounded; a GPU
/// counts the same in integers. An infinite alpha takes every finite color to 0 and an
/// infinite one to 255, by the same count.
///
/// Fast: an opaque color (alpha 1) is its own straight value ([`srgb8`]); any other starts
/// at the exact code or the one above it ([`straight_start`]), then moves down while the
/// code below is not reached, comparing exactly: one or two products decide it, whatever
/// the values (`starts_each_color_over_alpha_at_its_code_or_the_one_above`).
#[must_use]
fn straight8(color: f32, alpha: f32) -> u8 {
    if alpha.to_bits() == HIGH_BITS {
        return srgb8(color);
    }
    let (exact_color, exact_alpha) = (f64::from(color), f64::from(alpha));
    // Whether code `code` is reached: its threshold times alpha at most the color (none
    // past 255).
    let reaches = |code: u8| {
        SRGB8_THRESHOLDS
            .get(usize::from(code))
            .is_some_and(|&threshold| f64::from(threshold) * exact_alpha <= exact_color)
    };
    let mut code = straight_start(color, alpha);
    while let Some(below) = code.checked_sub(1).filter(|&below| !reaches(below)) {
        code = below;
    }
    code
}

/// Where [`straight8`] starts: the exact code or the one above it, never below.
///
/// The code of the rounded quotient, under any rounding mode a host may set: the rounded
/// quotient is the straight value (the largest float at or below the real quotient) or the
/// float above it, so its code is the straight value's or one above (one unit in the last
/// place, where two thresholds are thousands of units apart). Flushed to zero, a subnormal
/// color or quotient becomes 0, below the straight value but in the same code, 0: every
/// subnormal, and any subnormal color over an alpha whose code is above 0, is far below the
/// first threshold (2⁻¹³). An infinite color, whose quotient over an infinite
/// alpha is not a number (code 0), starts at 255: its code over any alpha, every threshold
/// times the alpha at most infinity. Proven against an independent count
/// (`starts_each_color_over_alpha_at_its_code_or_the_one_above`).
fn straight_start(color: f32, alpha: f32) -> u8 {
    if color.to_bits() == INFINITY_BITS {
        return 255;
    }
    srgb8(color / alpha)
}

/// The four display codes of one pixel of premultiplied linear values.
///
/// Each color over alpha, the largest float at or below the real quotient (`straight8`),
/// then alpha ([`alpha8`]); a pixel whose alpha encodes to 0 is transparent black.
#[must_use]
pub fn pixel8(red: f32, green: f32, blue: f32, alpha: f32) -> [u8; 4] {
    let opacity = alpha8(alpha);
    if opacity == 0 {
        return [0; 4];
    }
    [
        straight8(red, alpha),
        straight8(green, alpha),
        straight8(blue, alpha),
        opacity,
    ]
}

/// Encodes `values` into `bytes` for display, each pixel by [`pixel8`].
///
/// Four values a pixel (premultiplied linear red, green, blue, alpha), four bytes a pixel:
/// the one loop of every CPU encoder of the library ([`DisplayImage::encode`], and any values
/// a host gives); the GPU's encoder is its shader. As many whole pixels as both hold, which
/// it returns, so that a caller given slices of other lengths sees how many were encoded;
/// the bytes past them are left as they were. Allocates nothing.
#[must_use = "the whole pixels encoded: fewer than a slice holds when their lengths differ"]
pub fn encode_pixels(values: &[f32], bytes: &mut [u8]) -> usize {
    let (pixels, _) = bytes.as_chunks_mut::<{ DisplayImage::CHANNELS }>();
    let (values, _) = values.as_chunks::<{ Image::CHANNELS }>();
    let mut encoded = 0_usize;
    for (pixel, &[red, green, blue, alpha]) in pixels.iter_mut().zip(values) {
        *pixel = pixel8(red, green, blue, alpha);
        encoded = encoded.saturating_add(1);
    }
    encoded
}

/// An image in its display encoding: 8-bit sRGB, straight alpha, four bytes a pixel (red,
/// green, blue, alpha), row by row from the top, each row from the left: the layout of a
/// canvas's `ImageData`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayImage {
    /// Its size.
    size: ImageSize,
    /// The four bytes of every pixel, in order.
    bytes: Vec<u8>,
}

impl DisplayImage {
    /// The bytes per pixel: red, green, blue, alpha.
    pub const CHANNELS: usize = 4;

    /// The bytes per pixel, as a count of memory: [`DisplayImage::CHANNELS`].
    pub const PIXEL_BYTES: u64 = 4;

    /// A transparent black display image. Allocates all its memory now, so that encoding
    /// into it never allocates.
    ///
    /// # Errors
    ///
    /// Returns [`VisualError::OutOfMemory`], naming the size, when the machine cannot hold
    /// it (at most 128 MiB: [`ImageSize::MAX_PIXELS`] pixels of 4 bytes); never aborts,
    /// never traps.
    pub fn try_new(size: ImageSize) -> Result<Self, VisualError> {
        Self::allocate(size, Vec::try_reserve_exact)
    }

    /// [`DisplayImage::try_new`], reserving the bytes with `reserve`: the allocator's own,
    /// or one that refuses, in the tests (a function pointer, so that the body the tests
    /// cover is the one shipped).
    fn allocate(
        size: ImageSize,
        reserve: fn(&mut Vec<u8>, usize) -> Result<(), TryReserveError>,
    ) -> Result<Self, VisualError> {
        let length = usize::try_from(size.pixels())
            .unwrap_or(usize::MAX)
            .saturating_mul(Self::CHANNELS);
        let mut bytes = Vec::new();
        reserve(&mut bytes, length).map_err(|_| VisualError::OutOfMemory {
            width: size.width(),
            height: size.height(),
            of: Holding::DisplayEncoding,
        })?;
        // Within the reservation: no other allocation.
        bytes.resize(length, 0);
        Ok(Self { size, bytes })
    }

    /// Its size.
    #[must_use]
    pub const fn size(&self) -> ImageSize {
        self.size
    }

    /// Its bytes, row by row from the top, four per pixel.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Its bytes to write, for an encoder other than [`DisplayImage::encode`] (a GPU's):
    /// any bytes are a display image, so that nothing it writes breaks one.
    pub fn bytes_mut(&mut self) -> &mut [u8] {
        &mut self.bytes
    }

    /// Encodes `image` into it, every pixel by [`pixel8`] ([`encode_pixels`]); allocates
    /// nothing.
    ///
    /// # Errors
    ///
    /// Returns [`VisualError::DisplaySizeMismatch`], naming the image's size, when it is not
    /// of that size; it is then left as it was.
    pub fn encode(&mut self, image: &Image) -> Result<(), VisualError> {
        if image.size() != self.size {
            return Err(VisualError::DisplaySizeMismatch {
                width: image.size().width(),
                height: image.size().height(),
            });
        }
        // Of the same size, checked above: every pixel encoded.
        let _every: usize = encode_pixels(image.values(), &mut self.bytes);
        Ok(())
    }
}

#[cfg(test)]
#[path = "display_tests.rs"]
mod tests;
