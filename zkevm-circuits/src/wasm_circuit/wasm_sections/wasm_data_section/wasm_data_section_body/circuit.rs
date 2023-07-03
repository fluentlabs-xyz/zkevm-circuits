use halo2_proofs::{
    plonk::{Column, ConstraintSystem},
};
use std::{marker::PhantomData};
use std::rc::Rc;
use ethers_core::k256::pkcs8::der::Encode;
use halo2_proofs::circuit::{Region, Value};
use halo2_proofs::plonk::{Fixed, VirtualCells};
use halo2_proofs::poly::Rotation;
use log::debug;
use eth_types::Field;
use gadgets::util::{Expr, or};
use crate::evm_circuit::util::constraint_builder::{BaseConstraintBuilder, ConstrainBuilderCommon};
use crate::wasm_circuit::consts::NumericInstruction::I32Const;
use crate::wasm_circuit::consts::{MemSegmentType, WASM_BLOCK_END};
use crate::wasm_circuit::error::Error;
use crate::wasm_circuit::leb128_circuit::circuit::LEB128Chip;
use crate::wasm_circuit::leb128_circuit::helpers::{leb128_compute_sn, leb128_compute_sn_recovered_at_position};
use crate::wasm_circuit::wasm_bytecode::bytecode::WasmBytecode;
use crate::wasm_circuit::wasm_bytecode::bytecode_table::WasmBytecodeTable;
use crate::wasm_circuit::wasm_sections::consts::LebParams;
use crate::wasm_circuit::wasm_sections::helpers::configure_check_for_transition;

#[derive(Debug, Clone)]
pub struct WasmDataSectionBodyConfig<F: Field> {
    pub q_enable: Column<Fixed>,
    pub is_items_count: Column<Fixed>,
    pub is_mem_segment_type: Column<Fixed>,
    pub is_mem_segment_size_opcode: Column<Fixed>,
    pub is_mem_segment_size: Column<Fixed>,
    pub is_block_end: Column<Fixed>,
    pub is_mem_segment_len: Column<Fixed>,
    pub is_mem_segment_bytes: Column<Fixed>,

    pub leb128_chip: Rc<LEB128Chip<F>>,

    _marker: PhantomData<F>,
}

impl<'a, F: Field> WasmDataSectionBodyConfig<F>
{}

#[derive(Debug, Clone)]
pub struct WasmDataSectionBodyChip<F: Field> {
    pub config: WasmDataSectionBodyConfig<F>,
    _marker: PhantomData<F>,
}

impl<F: Field> WasmDataSectionBodyChip<F>
{
    pub fn construct(config: WasmDataSectionBodyConfig<F>) -> Self {
        let instance = Self {
            config,
            _marker: PhantomData,
        };
        instance
    }

    pub fn configure(
        cs: &mut ConstraintSystem<F>,
        bytecode_table: Rc<WasmBytecodeTable>,
        leb128_chip: Rc<LEB128Chip<F>>,
    ) -> WasmDataSectionBodyConfig<F> {
        let q_enable = cs.fixed_column();
        let is_items_count = cs.fixed_column();
        let is_mem_segment_type = cs.fixed_column();
        let is_mem_segment_size_opcode = cs.fixed_column();
        let is_mem_segment_size = cs.fixed_column();
        let is_block_end = cs.fixed_column();
        let is_mem_segment_len = cs.fixed_column();
        let is_mem_segment_bytes = cs.fixed_column();

        cs.create_gate("WasmDataSectionBody gate", |vc| {
            let mut cb = BaseConstraintBuilder::default();

            let q_enable_expr = vc.query_fixed(q_enable, Rotation::cur());
            let is_items_count_expr = vc.query_fixed(is_items_count, Rotation::cur());
            let is_mem_segment_type_expr = vc.query_fixed(is_mem_segment_type, Rotation::cur());
            let is_mem_segment_size_opcode_expr = vc.query_fixed(is_mem_segment_size_opcode, Rotation::cur());
            let is_mem_segment_size_expr = vc.query_fixed(is_mem_segment_size, Rotation::cur());
            let is_block_end_expr = vc.query_fixed(is_block_end, Rotation::cur());
            let is_mem_segment_len_expr = vc.query_fixed(is_mem_segment_len, Rotation::cur());
            let is_mem_segment_bytes_expr = vc.query_fixed(is_mem_segment_bytes, Rotation::cur());

            let byte_val_expr = vc.query_advice(bytecode_table.value, Rotation::cur());

            cb.require_boolean("q_enable is boolean", q_enable_expr.clone());
            cb.require_boolean("is_items_count is boolean", is_items_count_expr.clone());
            cb.require_boolean("is_mem_segment_type is boolean", is_mem_segment_type_expr.clone());
            cb.require_boolean("is_mem_segment_size_opcode is boolean", is_mem_segment_size_opcode_expr.clone());
            cb.require_boolean("is_mem_segment_size is boolean", is_mem_segment_size_expr.clone());
            cb.require_boolean("is_block_end_expr is boolean", is_block_end_expr.clone());
            cb.require_boolean("is_mem_segment_len is boolean", is_mem_segment_len_expr.clone());
            cb.require_boolean("is_mem_segment_bytes is boolean", is_mem_segment_bytes_expr.clone());

            cb.require_equal(
                "exactly one mark flag active at the same time",
                is_items_count_expr.clone()
                    + is_mem_segment_type_expr.clone()
                    + is_mem_segment_size_opcode_expr.clone()
                    + is_mem_segment_size_expr.clone()
                    + is_block_end_expr.clone()
                    + is_mem_segment_len_expr.clone()
                    + is_mem_segment_bytes_expr.clone()
                ,
                1.expr(),
            );

            cb.condition(
                or::expr([
                    is_items_count_expr.clone(),
                    is_mem_segment_size_expr.clone(),
                    is_mem_segment_len_expr.clone(),
                ]),
                |cbc| {
                    cbc.require_equal(
                        "is_items_count || is_mem_segment_size || is_mem_segment_len -> leb128",
                        vc.query_fixed(leb128_chip.config.q_enable, Rotation::cur()),
                        1.expr(),
                    )
                }
            );

            // is_items_count+ -> item+ (is_mem_segment_type{1} -> is_mem_segment_size_opcode{1} -> is_mem_segment_size+ -> is_block_end_expr{1} -> is_mem_segment_len+ -> is_mem_segment_bytes+)
            configure_check_for_transition(
                &mut cb,
                vc,
                "check next: is_items_count+ -> item+ (is_mem_segment_type{1} ...",
                is_items_count_expr.clone(),
                true,
                &[is_items_count, is_mem_segment_type],
            );
            configure_check_for_transition(
                &mut cb,
                vc,
                "check prev: is_items_count+ -> item+ (is_mem_segment_type{1} ...",
                is_mem_segment_type_expr.clone(),
                false,
                &[is_items_count, is_mem_segment_bytes, is_mem_segment_type],
            );
            configure_check_for_transition(
                &mut cb,
                vc,
                "check next: is_mem_segment_type{1} -> is_mem_segment_size_opcode{1}",
                is_mem_segment_type_expr.clone(),
                true,
                &[is_mem_segment_size_opcode],
            );
            configure_check_for_transition(
                &mut cb,
                vc,
                "check prev: is_mem_segment_type{1} -> is_mem_segment_size_opcode{1}",
                is_mem_segment_size_opcode_expr.clone(),
                false,
                &[is_mem_segment_type],
            );
            configure_check_for_transition(
                &mut cb,
                vc,
                "check next: is_mem_segment_size_opcode{1} -> is_mem_segment_size+",
                is_mem_segment_size_opcode_expr.clone(),
                true,
                &[is_mem_segment_size],
            );
            configure_check_for_transition(
                &mut cb,
                vc,
                "check prev: is_mem_segment_size_opcode{1} -> is_mem_segment_size+",
                is_mem_segment_size_expr.clone(),
                false,
                &[is_mem_segment_size_opcode, is_mem_segment_size],
            );
            configure_check_for_transition(
                &mut cb,
                vc,
                "check next: is_mem_segment_size+ -> is_block_end_expr{1}",
                is_mem_segment_size_expr.clone(),
                true,
                &[is_mem_segment_size, is_block_end],
            );
            configure_check_for_transition(
                &mut cb,
                vc,
                "check prev: is_mem_segment_size+ -> is_block_end_expr{1}",
                is_block_end_expr.clone(),
                false,
                &[is_mem_segment_size],
            );
            configure_check_for_transition(
                &mut cb,
                vc,
                "check next: is_block_end_expr{1} -> is_mem_segment_len+",
                is_block_end_expr.clone(),
                true,
                &[is_mem_segment_len],
            );
            configure_check_for_transition(
                &mut cb,
                vc,
                "check prev: is_block_end_expr{1} -> is_mem_segment_len+",
                is_mem_segment_len_expr.clone(),
                false,
                &[is_block_end, is_mem_segment_len],
            );
            configure_check_for_transition(
                &mut cb,
                vc,
                "check next: is_mem_segment_len+ -> is_mem_segment_bytes+",
                is_mem_segment_len_expr.clone(),
                true,
                &[is_mem_segment_len, is_mem_segment_bytes],
            );
            configure_check_for_transition(
                &mut cb,
                vc,
                "check prev: is_mem_segment_len+ -> is_mem_segment_bytes+",
                is_mem_segment_bytes_expr.clone(),
                false,
                &[is_mem_segment_len, is_mem_segment_bytes],
            );

            cb.condition(
                is_block_end_expr.clone(),
                |bcb| {
                    bcb.require_equal(
                        "is_block_end_expr -> byte value == WASM_BLOCK_END",
                        byte_val_expr.clone(),
                        WASM_BLOCK_END.expr(),
                    )
                }
            );

            cb.condition(
                is_mem_segment_type_expr.clone(),
                |bcb| {
                    bcb.require_in_set(
                        "is_mem_segment_type -> byte value is correct",
                        byte_val_expr.clone(),
                        vec![
                            (MemSegmentType::ActiveZero as i32).expr(),
                            // TODO add support for other types
                            // (MemSegmentType::Passive as i32).expr(),
                            // (MemSegmentType::ActiveVariadic as i32).expr(),
                        ],
                    )
                }
            );

            cb.condition(
                is_mem_segment_size_opcode_expr.clone(),
                |bcb| {
                    bcb.require_in_set(
                        "is_mem_segment_size_opcode -> byte value is correct",
                        byte_val_expr.clone(),
                        vec![
                            (I32Const as i32).expr(),
                            // TODO add support for other types?
                        ],
                    )
                }
            );

            cb.gate(q_enable_expr.clone())
        });

        let config = WasmDataSectionBodyConfig::<F> {
            q_enable,
            is_items_count,
            is_mem_segment_type,
            is_mem_segment_size_opcode,
            is_mem_segment_size,
            is_block_end,
            is_mem_segment_len,
            is_mem_segment_bytes,
            leb128_chip,
            _marker: PhantomData,
        };

        config
    }

    pub fn assign(
        &self,
        region: &mut Region<F>,
        offset: usize,
        is_items_count: bool,
        is_mem_segment_type: bool,
        is_mem_segment_size_opcode: bool,
        is_mem_segment_size: bool,
        is_block_end_expr: bool,
        is_mem_segment_len: bool,
        is_mem_segment_bytes: bool,
        leb_params: Option<LebParams>,
    ) {
        let q_enable = true;
        debug!(
            "assign at offset {} \
            q_enable {} \
            is_items_count {} \
            is_mem_segment_type {} \
            is_mem_segment_size_opcode {} \
            is_mem_segment_size {} \
            is_block_end_expr {} \
            is_mem_segment_len {} \
            is_mem_segment_bytes {} \
            ",
            offset,
            q_enable,
            is_items_count,
            is_mem_segment_type,
            is_mem_segment_size_opcode,
            is_mem_segment_size,
            is_block_end_expr,
            is_mem_segment_len,
            is_mem_segment_bytes,
        );
        if is_items_count || is_mem_segment_size || is_mem_segment_len {
            let p = leb_params.unwrap();
            self.config.leb128_chip.assign(
                region,
                offset,
                q_enable,
                p,
            );
        }
        region.assign_fixed(
            || format!("assign 'q_enable' val {} at {}", q_enable, offset),
            self.config.q_enable,
            offset,
            || Value::known(F::from(q_enable as u64)),
        ).unwrap();
        region.assign_fixed(
            || format!("assign 'is_items_count' val {} at {}", is_items_count, offset),
            self.config.is_items_count,
            offset,
            || Value::known(F::from(is_items_count as u64)),
        ).unwrap();
        region.assign_fixed(
            || format!("assign 'is_mem_segment_type' val {} at {}", is_mem_segment_type, offset),
            self.config.is_mem_segment_type,
            offset,
            || Value::known(F::from(is_mem_segment_type as u64)),
        ).unwrap();
        region.assign_fixed(
            || format!("assign 'is_mem_segment_size_opcode' val {} at {}", is_mem_segment_size_opcode, offset),
            self.config.is_mem_segment_size_opcode,
            offset,
            || Value::known(F::from(is_mem_segment_size_opcode as u64)),
        ).unwrap();
        region.assign_fixed(
            || format!("assign 'is_mem_segment_size' val {} at {}", is_mem_segment_size, offset),
            self.config.is_mem_segment_size,
            offset,
            || Value::known(F::from(is_mem_segment_size as u64)),
        ).unwrap();
        region.assign_fixed(
            || format!("assign 'is_block_end_expr' val {} at {}", is_block_end_expr, offset),
            self.config.is_block_end,
            offset,
            || Value::known(F::from(is_block_end_expr as u64)),
        ).unwrap();
        region.assign_fixed(
            || format!("assign 'is_mem_segment_len' val {} at {}", is_mem_segment_len, offset),
            self.config.is_mem_segment_len,
            offset,
            || Value::known(F::from(is_mem_segment_len as u64)),
        ).unwrap();
        region.assign_fixed(
            || format!("assign 'is_mem_segment_bytes' val {} at {}", is_mem_segment_bytes, offset),
            self.config.is_mem_segment_bytes,
            offset,
            || Value::known(F::from(is_mem_segment_bytes as u64)),
        ).unwrap();
    }

    /// returns sn and leb len
    fn markup_leb_section(
        &self,
        region: &mut Region<F>,
        leb_bytes: &[u8],
        leb_bytes_start_offset: usize,
        is_items_count: bool,
        is_mem_segment_size: bool,
        is_mem_segment_len: bool,
    ) -> (u64, usize) {
        const OFFSET: usize = 0;
        let is_signed_leb = false;
        let (sn, last_byte_rel_offset) = leb128_compute_sn(leb_bytes, is_signed_leb, OFFSET).unwrap();
        let mut sn_recovered_at_pos = 0;
        for byte_rel_offset in OFFSET..=last_byte_rel_offset {
            let offset = leb_bytes_start_offset + byte_rel_offset;
            sn_recovered_at_pos = leb128_compute_sn_recovered_at_position(
                sn_recovered_at_pos,
                false,
                byte_rel_offset,
                last_byte_rel_offset,
                leb_bytes[byte_rel_offset],
            );
            self.assign(
                region,
                offset,
                is_items_count,
                false,
                false,
                is_mem_segment_size,
                false,
                is_mem_segment_len,
                false,
                Some(LebParams{
                    is_signed: is_signed_leb,
                    byte_rel_offset,
                    last_byte_rel_offset,
                    sn,
                    sn_recovered_at_pos,
                }),
            );
        }

        (sn, last_byte_rel_offset + 1)
    }

    /// returns new offset
    fn markup_bytes_section(
        &self,
        region: &mut Region<F>,
        offset: usize,
        len: usize,
    ) -> usize {
        for (_rel_offset, byte_offset) in (offset..offset + len).enumerate() {
            self.assign(
                region,
                byte_offset,
                false,
                false,
                false,
                false,
                false,
                false,
                true,
                None,
            );
        }
        offset + len
    }

    /// returns new offset
    pub fn assign_auto(
        &self,
        region: &mut Region<F>,
        wasm_bytecode: &WasmBytecode,
        offset_start: usize,
    ) -> Result<usize, Error> {
        let mut offset = offset_start;

        let (items_count, items_count_leb_len) = self.markup_leb_section(
            region,
            &wasm_bytecode.bytes.as_slice()[offset..],
            offset,
            true,
            false,
            false,
        );
        offset += items_count_leb_len;

        for _item_index in 0..items_count {

            // is_mem_segment_type{1}
            self.assign(
                region,
                offset,
                false,
                true,
                false,
                false,
                false,
                false,
                false,
                None,
            );
            offset += 1;

            // is_mem_segment_size_opcode{1}
            self.assign(
                region,
                offset,
                false,
                false,
                true,
                false,
                false,
                false,
                false,
                None,
            );
            offset += 1;

            // is_mem_segment_size+
            let (mem_segment_size, mem_segment_size_leb_len) = self.markup_leb_section(
                region,
                &wasm_bytecode.bytes.as_slice()[offset..],
                offset,
                false,
                true,
                false,
            );
            offset += mem_segment_size_leb_len;

            // is_block_end_expr{1}
            self.assign(
                region,
                offset,
                false,
                false,
                false,
                false,
                true,
                false,
                false,
                None,
            );
            offset += 1;

            // is_mem_segment_len+
            let (mem_segment_len, mem_segment_len_leb_len) = self.markup_leb_section(
                region,
                &wasm_bytecode.bytes.as_slice()[offset..],
                offset,
                false,
                false,
                true,
            );
            offset += mem_segment_len_leb_len;

            // is_mem_segment_bytes+
            for rel_offset in 0..(mem_segment_len as usize) {
                self.assign(
                    region,
                    offset + rel_offset,
                    false,
                    false,
                    false,
                    false,
                    false,
                    false,
                    true,
                    None,
                );
            }
            offset += mem_segment_len as usize;
        }

        Ok(offset)
    }
}