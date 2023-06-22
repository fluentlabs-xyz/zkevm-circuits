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
use crate::wasm_circuit::consts::{NumType, WASM_EXPR_DELIMITER};
use crate::wasm_circuit::consts::NumericInstruction::{F32Const, F64Const, I32Const, I64Const};
use crate::wasm_circuit::error::Error;
use crate::wasm_circuit::leb128_circuit::circuit::LEB128Chip;
use crate::wasm_circuit::leb128_circuit::helpers::{leb128_compute_sn, leb128_compute_sn_recovered_at_position};
use crate::wasm_circuit::wasm_bytecode::bytecode::WasmBytecode;
use crate::wasm_circuit::wasm_bytecode::bytecode_table::WasmBytecodeTable;
use crate::wasm_circuit::wasm_sections::helpers::configure_check_for_transition;

#[derive(Debug, Clone)]
pub struct WasmGlobalSectionBodyConfig<F: Field> {
    pub q_enable: Column<Fixed>,
    pub is_items_count: Column<Fixed>,
    pub is_global_type: Column<Fixed>,
    pub is_mut_prop: Column<Fixed>,
    pub is_init_opcode: Column<Fixed>,
    pub is_init_val: Column<Fixed>,
    pub is_expr_delimiter: Column<Fixed>,

    pub leb128_chip: Rc<LEB128Chip<F>>,

    _marker: PhantomData<F>,
}

impl<'a, F: Field> WasmGlobalSectionBodyConfig<F>
{}

#[derive(Debug, Clone)]
pub struct WasmGlobalSectionBodyChip<F: Field> {
    pub config: WasmGlobalSectionBodyConfig<F>,
    _marker: PhantomData<F>,
}

impl<F: Field> WasmGlobalSectionBodyChip<F>
{
    pub fn construct(config: WasmGlobalSectionBodyConfig<F>) -> Self {
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
    ) -> WasmGlobalSectionBodyConfig<F> {
        let q_enable = cs.fixed_column();
        let is_items_count = cs.fixed_column();
        let is_global_type = cs.fixed_column();
        let is_mut_prop = cs.fixed_column();
        let is_init_opcode = cs.fixed_column();
        let is_init_val = cs.fixed_column();
        let is_expr_delimiter = cs.fixed_column();

        cs.create_gate("WasmGlobalSectionBody gate", |vc| {
            let mut cb = BaseConstraintBuilder::default();

            let q_enable_expr = vc.query_fixed(q_enable, Rotation::cur());
            let is_items_count_expr = vc.query_fixed(is_items_count, Rotation::cur());
            let is_global_type_expr = vc.query_fixed(is_global_type, Rotation::cur());
            let is_mut_prop_expr = vc.query_fixed(is_mut_prop, Rotation::cur());
            let is_init_opcode_expr = vc.query_fixed(is_init_opcode, Rotation::cur());
            let is_init_val_expr = vc.query_fixed(is_init_val, Rotation::cur());
            let is_expr_delimiter_expr = vc.query_fixed(is_expr_delimiter, Rotation::cur());

            let byte_val_expr = vc.query_advice(bytecode_table.value, Rotation::cur());

            cb.require_boolean("q_enable is boolean", q_enable_expr.clone());
            cb.require_boolean("is_items_count is boolean", is_items_count_expr.clone());
            cb.require_boolean("is_global_type is boolean", is_global_type_expr.clone());
            cb.require_boolean("is_mut_prop is boolean", is_mut_prop_expr.clone());
            cb.require_boolean("is_init_opcode is boolean", is_init_opcode_expr.clone());
            cb.require_boolean("is_init_val is boolean", is_init_val_expr.clone());
            cb.require_boolean("is_expr_delimiter is boolean", is_expr_delimiter_expr.clone());

            cb.require_equal(
                "exactly one mark flag active at the same time",
                is_items_count_expr.clone()
                    + is_global_type_expr.clone()
                    + is_mut_prop_expr.clone()
                    + is_init_opcode_expr.clone()
                    + is_init_val_expr.clone()
                    + is_expr_delimiter_expr.clone()
                ,
                1.expr(),
            );

            cb.condition(
                or::expr([
                    is_items_count_expr.clone(),
                    is_init_val_expr.clone(),
                ]),
                |cbc| {
                    cbc.require_equal(
                        "is_items_count || is_init_val -> leb128",
                        vc.query_fixed(leb128_chip.config.q_enable, Rotation::cur()),
                        1.expr(),
                    )
                }
            );

            // is_items_count+ -> item+(is_global_type{1} -> is_mut_prop{1} -> is_init_opcode{1} -> is_init_val+ -> is_expr_delimiter{1})
            configure_check_for_transition(
                &mut cb,
                vc,
                "check next: is_items_count+ -> item+(is_global_type{1} ...",
                is_items_count_expr.clone(),
                true,
                &[is_items_count, is_global_type, ],
            );
            configure_check_for_transition(
                &mut cb,
                vc,
                "check prev: is_items_count+ -> item+(is_global_type{1} ...",
                is_global_type_expr.clone(),
                false,
                &[is_items_count, is_expr_delimiter, ],
            );
            configure_check_for_transition(
                &mut cb,
                vc,
                "check next: is_global_type{1} -> is_mut_prop{1}",
                is_global_type_expr.clone(),
                true,
                &[is_mut_prop, ],
            );
            configure_check_for_transition(
                &mut cb,
                vc,
                "check prev: is_global_type{1} -> is_mut_prop{1}",
                is_mut_prop_expr.clone(),
                false,
                &[is_global_type, ],
            );
            configure_check_for_transition(
                &mut cb,
                vc,
                "check next: is_mut_prop{1} -> is_init_opcode{1}",
                is_mut_prop_expr.clone(),
                true,
                &[is_init_opcode, ],
            );
            configure_check_for_transition(
                &mut cb,
                vc,
                "check prev: is_mut_prop{1} -> is_init_opcode{1}",
                is_init_opcode_expr.clone(),
                false,
                &[is_mut_prop, ],
            );
            configure_check_for_transition(
                &mut cb,
                vc,
                "check next: is_init_opcode{1} -> is_init_val+",
                is_init_opcode_expr.clone(),
                true,
                &[is_init_val, ],
            );
            configure_check_for_transition(
                &mut cb,
                vc,
                "check prev: is_init_opcode{1} -> is_init_val+",
                is_init_val_expr.clone(),
                false,
                &[is_init_opcode, is_init_val, ],
            );
            configure_check_for_transition(
                &mut cb,
                vc,
                "check next: is_init_val+ -> is_expr_delimiter{1}",
                is_init_val_expr.clone(),
                true,
                &[is_init_val, is_expr_delimiter],
            );
            configure_check_for_transition(
                &mut cb,
                vc,
                "check prev: is_init_val+ -> is_expr_delimiter{1}",
                is_expr_delimiter_expr.clone(),
                false,
                &[is_init_val, ],
            );

            cb.condition(
                is_global_type_expr.clone(),
                |bcb| {
                    bcb.require_in_set(
                        "is_global_type has eligible byte value",
                        byte_val_expr.clone(),
                        vec![
                            (NumType::I32 as i32).expr(),
                            (NumType::I64 as i32).expr(),
                            // TODO add support for float types
                            // (NumType::F32 as i32).expr(),
                            // (NumType::F64 as i32).expr(),
                        ],
                    )
                }
            );

            cb.condition(
                is_mut_prop_expr.clone(),
                |bcb| {
                    bcb.require_boolean(
                        "is_mut_prop -> bool",
                        byte_val_expr.clone(),
                    )
                }
            );

            cb.condition(
                is_init_opcode_expr.clone(),
                |bcb| {
                    bcb.require_in_set(
                        "is_init_opcode has eligible byte value",
                        byte_val_expr.clone(),
                        vec![
                            (I32Const as i32).expr(),
                            (I64Const as i32).expr(),
                            // TODO add support for float types
                            // (F32Const as i32).expr(),
                            // (F64Const as i32).expr(),
                        ],
                    )
                }
            );

            // TODO constraint is_global_type_expr based on is_init_opcode_expr

            cb.condition(
                is_expr_delimiter_expr.clone(),
                |bcb| {
                    bcb.require_equal(
                        "is_expr_delimiter -> byte value == WASM_EXPR_DELIMITER",
                        byte_val_expr.clone(),
                        WASM_EXPR_DELIMITER.expr(),
                    )
                }
            );

            cb.gate(q_enable_expr.clone())
        });

        let config = WasmGlobalSectionBodyConfig::<F> {
            q_enable,
            is_items_count,
            is_global_type,
            is_mut_prop,
            is_init_opcode,
            is_init_val,
            is_expr_delimiter,
            leb128_chip,
            _marker: PhantomData,
        };

        config
    }

    pub fn assign_init(
        &self,
        region: &mut Region<F>,
        offset_max: usize,
    ) {
        for offset in 0..=offset_max {
            self.assign(
                region,
                offset,
                false,
                false,
                false,
                false,
                false,
                false,
                0,
                0,
                0,
                0,
            );
        }
    }

    pub fn assign(
        &self,
        region: &mut Region<F>,
        offset: usize,
        is_items_count: bool,
        is_global_type: bool,
        is_mut_prop: bool,
        is_init_opcode: bool,
        is_init_val: bool,
        is_expr_delimiter: bool,
        leb_byte_rel_offset: usize,
        leb_last_byte_rel_offset: usize,
        leb_sn: u64,
        leb_sn_recovered_at_pos: u64,
    ) {
        let q_enable = is_items_count || is_global_type || is_mut_prop || is_init_opcode || is_init_val || is_expr_delimiter;
        debug!(
            "offset {} q_enable {} is_items_count {} is_global_type {} is_mut_prop {} is_init_opcode {} is_init_val {} is_expr_delimiter {}",
            offset,
            q_enable,
            is_items_count,
            is_global_type,
            is_mut_prop,
            is_init_opcode,
            is_init_val,
            is_expr_delimiter,
        );
        if is_items_count || is_init_val {
            let is_first_leb_byte = leb_byte_rel_offset == 0;
            let is_last_leb_byte = leb_byte_rel_offset == leb_last_byte_rel_offset;
            let is_leb_byte_has_cb = leb_byte_rel_offset < leb_last_byte_rel_offset;
            self.config.leb128_chip.assign(
                region,
                offset,
                leb_byte_rel_offset,
                q_enable,
                is_first_leb_byte,
                is_last_leb_byte,
                is_leb_byte_has_cb,
                false,
                leb_sn,
                leb_sn_recovered_at_pos,
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
            || format!("assign 'is_global_type' val {} at {}", is_global_type, offset),
            self.config.is_global_type,
            offset,
            || Value::known(F::from(is_global_type as u64)),
        ).unwrap();
        region.assign_fixed(
            || format!("assign 'is_mut_prop' val {} at {}", is_mut_prop, offset),
            self.config.is_mut_prop,
            offset,
            || Value::known(F::from(is_mut_prop as u64)),
        ).unwrap();
        region.assign_fixed(
            || format!("assign 'is_init_opcode' val {} at {}", is_init_opcode, offset),
            self.config.is_init_opcode,
            offset,
            || Value::known(F::from(is_init_opcode as u64)),
        ).unwrap();
        region.assign_fixed(
            || format!("assign 'is_init_val' val {} at {}", is_init_val, offset),
            self.config.is_init_val,
            offset,
            || Value::known(F::from(is_init_val as u64)),
        ).unwrap();
        region.assign_fixed(
            || format!("assign 'is_expr_delimiter' val {} at {}", is_expr_delimiter, offset),
            self.config.is_expr_delimiter,
            offset,
            || Value::known(F::from(is_expr_delimiter as u64)),
        ).unwrap();
    }

    /// returns sn and leb len
    fn markup_leb_section(
        &self,
        region: &mut Region<F>,
        leb_bytes: &[u8],
        leb_bytes_start_offset: usize,
        is_items_count: bool,
        is_init_val: bool,
    ) -> (u64, usize) {
        const OFFSET: usize = 0;
        let (leb_sn, last_byte_offset) = leb128_compute_sn(leb_bytes, false, OFFSET).unwrap();
        let mut leb_sn_recovered_at_pos = 0;
        for byte_offset in OFFSET..=last_byte_offset {
            leb_sn_recovered_at_pos = leb128_compute_sn_recovered_at_position(
                leb_sn_recovered_at_pos,
                false,
                byte_offset,
                last_byte_offset,
                leb_bytes[byte_offset],
            );
            let offset = leb_bytes_start_offset + byte_offset;
            self.assign(
                region,
                offset,
                is_items_count,
                false,
                false,
                false,
                is_init_val,
                false,
                byte_offset,
                last_byte_offset,
                leb_sn,
                leb_sn_recovered_at_pos,
            );
        }

        (leb_sn, last_byte_offset + 1)
    }

    /// returns new offset
    pub fn assign_auto(
        &self,
        region: &mut Region<F>,
        wasm_bytecode: &WasmBytecode,
        offset_start: usize,
    ) -> Result<usize, Error> {
        let mut offset = offset_start;
        debug!("offset_start {}", offset);

        let (items_count, items_count_leb_len) = self.markup_leb_section(
            region,
            &wasm_bytecode.bytes.as_slice()[offset..],
            offset,
            true,
            false,
        );
        debug!("offset {} items_count {} items_count_leb_len {}", offset, items_count, items_count_leb_len);
        offset += items_count_leb_len;

        for _item_index in 0..items_count {
            // is_global_type{1}
            self.assign(
                region,
                offset,
                false,
                true,
                false,
                false,
                false,
                false,
                0,
                0,
                0,
                0,
            );
            debug!("offset {} is_global_type", offset);
            offset += 1;

            // is_mut_prop{1}
            self.assign(
                region,
                offset,
                false,
                false,
                true,
                false,
                false,
                false,
                0,
                0,
                0,
                0,
            );
            debug!("offset {} is_mut_prop", offset);
            offset += 1;

            // is_init_opcode{1}
            self.assign(
                region,
                offset,
                false,
                false,
                false,
                true,
                false,
                false,
                0,
                0,
                0,
                0,
            );
            debug!("offset {} is_init_opcode", offset);
            offset += 1;

            // is_init_val+
            let (init_val, init_val_leb_len) = self.markup_leb_section(
                region,
                &wasm_bytecode.bytes.as_slice()[offset..],
                offset,
                false,
                true,
            );
            debug!("offset {} init_val {} init_val_leb_len {}", offset, init_val, init_val_leb_len);
            offset += init_val_leb_len;

            // is_expr_delimiter{1}
            self.assign(
                region,
                offset,
                false,
                false,
                false,
                false,
                false,
                true,
                0,
                0,
                0,
                0,
            );
            debug!("offset {} is_expr_delimiter", offset);
            offset += 1;
        }

        Ok(offset)
    }
}