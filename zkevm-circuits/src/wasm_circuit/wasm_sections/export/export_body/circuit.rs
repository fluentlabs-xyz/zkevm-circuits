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
use crate::wasm_circuit::error::Error;
use crate::wasm_circuit::leb128_circuit::circuit::LEB128Chip;
use crate::wasm_circuit::leb128_circuit::helpers::{leb128_compute_sn, leb128_compute_sn_recovered_at_position};
use crate::wasm_circuit::wasm_bytecode::bytecode::WasmBytecode;
use crate::wasm_circuit::wasm_bytecode::bytecode_table::WasmBytecodeTable;
use crate::wasm_circuit::wasm_sections::consts::LebParams;
use crate::wasm_circuit::wasm_sections::helpers::configure_check_for_transition;
use crate::wasm_circuit::wasm_sections::export::export_body::consts::ExportDesc;
use crate::wasm_circuit::wasm_sections::export::export_body::types::AssignType;

#[derive(Debug, Clone)]
pub struct WasmExportSectionBodyConfig<F: Field> {
    pub q_enable: Column<Fixed>,
    pub is_items_count: Column<Fixed>,
    pub is_export_name_len: Column<Fixed>,
    pub is_export_name: Column<Fixed>,
    pub is_exportdesc_type: Column<Fixed>,
    pub is_exportdesc_val: Column<Fixed>,

    pub leb128_chip: Rc<LEB128Chip<F>>,

    _marker: PhantomData<F>,
}

impl<'a, F: Field> WasmExportSectionBodyConfig<F>
{}

#[derive(Debug, Clone)]
pub struct WasmExportSectionBodyChip<F: Field> {
    pub config: WasmExportSectionBodyConfig<F>,
    _marker: PhantomData<F>,
}

impl<F: Field> WasmExportSectionBodyChip<F>
{
    pub fn construct(config: WasmExportSectionBodyConfig<F>) -> Self {
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
    ) -> WasmExportSectionBodyConfig<F> {
        let q_enable = cs.fixed_column();
        let is_items_count = cs.fixed_column();
        let is_export_name_len = cs.fixed_column();
        let is_export_name = cs.fixed_column();
        let is_exportdesc_type = cs.fixed_column();
        let is_exportdesc_val = cs.fixed_column();

        cs.create_gate("WasmExportSectionBody gate", |vc| {
            let mut cb = BaseConstraintBuilder::default();

            let q_enable_expr = vc.query_fixed(q_enable, Rotation::cur());
            let is_items_count_expr = vc.query_fixed(is_items_count, Rotation::cur());
            let is_export_name_len_expr = vc.query_fixed(is_export_name_len, Rotation::cur());
            let is_export_name_expr = vc.query_fixed(is_export_name, Rotation::cur());
            let is_exportdesc_type_expr = vc.query_fixed(is_exportdesc_type, Rotation::cur());
            let is_exportdesc_val_expr = vc.query_fixed(is_exportdesc_val, Rotation::cur());

            let byte_val_expr = vc.query_advice(bytecode_table.value, Rotation::cur());

            cb.require_boolean("q_enable is boolean", q_enable_expr.clone());
            cb.require_boolean("is_items_count is boolean", is_items_count_expr.clone());
            cb.require_boolean("is_export_name_len is boolean", is_export_name_len_expr.clone());
            cb.require_boolean("is_export_name is boolean", is_export_name_expr.clone());
            cb.require_boolean("is_exportdesc_type is boolean", is_exportdesc_type_expr.clone());
            cb.require_boolean("is_exportdesc_val is boolean", is_exportdesc_val_expr.clone());

            cb.require_equal(
                "exactly one mark flag active at the same time",
                is_items_count_expr.clone() + is_export_name_len_expr.clone() + is_export_name_expr.clone() + is_exportdesc_type_expr.clone() + is_exportdesc_val_expr.clone(),
                1.expr(),
            );

            cb.condition(
                or::expr([
                    is_items_count_expr.clone(),
                    is_export_name_len_expr.clone(),
                    is_exportdesc_val_expr.clone(),
                ]),
                |bcb| {
                    bcb.require_equal(
                        "is_items_count || is_export_name_len || is_exportdesc_val -> leb128",
                        vc.query_fixed(leb128_chip.config.q_enable, Rotation::cur()),
                        1.expr(),
                    )
                }
            );

            // is_items_count+ -> item+(is_export_name_len+ -> is_export_name+ -> is_exportdesc_type{1} -> is_exportdesc_val+)
            configure_check_for_transition(
                &mut cb,
                vc,
                "check next: is_items_count+ -> item+(is_export_name_len+ ...",
                is_items_count_expr.clone(),
                true,
                &[is_items_count, is_export_name_len],
            );
            configure_check_for_transition(
                &mut cb,
                vc,
                "check prev: is_items_count+ -> item+(is_export_name_len+ ...",
                is_export_name_len_expr.clone(),
                false,
                &[is_items_count, is_exportdesc_val, is_export_name_len],
            );
            configure_check_for_transition(
                &mut cb,
                vc,
                "check next: is_export_name_len+ -> is_export_name+",
                is_export_name_len_expr.clone(),
                true,
                &[is_export_name_len, is_export_name],
            );
            configure_check_for_transition(
                &mut cb,
                vc,
                "check prev: is_export_name_len+ -> is_export_name+",
                is_export_name_expr.clone(),
                false,
                &[is_export_name_len, is_export_name],
            );
            configure_check_for_transition(
                &mut cb,
                vc,
                "check next: is_export_name+ -> is_exportdesc_type{1}",
                is_export_name_expr.clone(),
                true,
                &[is_export_name, is_exportdesc_type],
            );
            configure_check_for_transition(
                &mut cb,
                vc,
                "check prev: is_export_name+ -> is_exportdesc_type{1}",
                is_exportdesc_type_expr.clone(),
                false,
                &[is_export_name],
            );
            configure_check_for_transition(
                &mut cb,
                vc,
                "check next: is_exportdesc_type{1} -> is_exportdesc_val+",
                is_exportdesc_type_expr.clone(),
                true,
                &[is_exportdesc_val],
            );
            configure_check_for_transition(
                &mut cb,
                vc,
                "check prev: is_exportdesc_type{1} -> is_exportdesc_val+",
                is_exportdesc_val_expr.clone(),
                false,
                &[is_exportdesc_type, is_exportdesc_val],
            );

            cb.condition(
                is_exportdesc_type_expr.clone(),
                |bcb| {
                    bcb.require_in_set(
                        "is_exportdesc_type -> byte_val has valid value",
                        byte_val_expr.clone(),
                        vec![
                            (ExportDesc::FuncExportDesc as i32).expr(),
                            (ExportDesc::TableExportDesc as i32).expr(),
                            (ExportDesc::MemExportDesc as i32).expr(),
                            (ExportDesc::GlobalExportDesc as i32).expr(),
                        ],
                    );
                }
            );

            cb.gate(q_enable_expr.clone())
        });

        let config = WasmExportSectionBodyConfig::<F> {
            q_enable,
            is_items_count,
            is_export_name_len,
            is_export_name,
            is_exportdesc_type,
            is_exportdesc_val,
            leb128_chip,
            _marker: PhantomData,
        };

        config
    }

    pub fn assign(
        &self,
        region: &mut Region<F>,
        wasm_bytecode: &WasmBytecode,
        offset: usize,
        assign_type: AssignType,
        assign_value: u64,
        leb_params: Option<LebParams>,
    ) {
        let q_enable = true;
        debug!(
            "export_section_body: assign at offset {} q_enable {} assign_type {:?} assign_value {} byte_val {:x?}",
            offset,
            q_enable,
            assign_type,
            assign_value,
            wasm_bytecode.bytes[offset],
        );
        if [
            AssignType::IsItemsCount,
            AssignType::IsExportNameLen,
            AssignType::IsExportdescVal,
        ].contains(&assign_type) {
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
        match assign_type {
            AssignType::IsItemsCount => {
                region.assign_fixed(
                    || format!("assign 'is_items_count' val {} at {}", assign_value, offset),
                    self.config.is_items_count,
                    offset,
                    || Value::known(F::from(assign_value)),
                ).unwrap();
            }
            AssignType::IsExportNameLen => {
                region.assign_fixed(
                    || format!("assign 'is_export_name_len' val {} at {}", assign_value, offset),
                    self.config.is_export_name_len,
                    offset,
                    || Value::known(F::from(assign_value)),
                ).unwrap();
            }
            AssignType::IsExportName => {
                region.assign_fixed(
                    || format!("assign 'is_export_name' val {} at {}", assign_value, offset),
                    self.config.is_export_name,
                    offset,
                    || Value::known(F::from(assign_value)),
                ).unwrap();
            }
            AssignType::IsExportdescType => {
                region.assign_fixed(
                    || format!("assign 'is_exportdesc_type' val {} at {}", assign_value, offset),
                    self.config.is_exportdesc_type,
                    offset,
                    || Value::known(F::from(assign_value)),
                ).unwrap();
            }
            AssignType::IsExportdescVal => {
                region.assign_fixed(
                    || format!("assign 'is_exportdesc_val' val {} at {}", assign_value, offset),
                    self.config.is_exportdesc_val,
                    offset,
                        || Value::known(F::from(assign_value)),
                ).unwrap();
            }
        }
    }

    /// returns new offset
    fn markup_name_section(
        &self,
        region: &mut Region<F>,
        wasm_bytecode: &WasmBytecode,
        offset: usize,
        assign_type: AssignType,
        name_len: usize,
    ) -> usize {
        for rel_offset in 0..name_len {
            self.assign(
                region,
                wasm_bytecode,
                offset + rel_offset,
                assign_type,
                1,
                None,
            );
        }
        offset + name_len
    }

    /// returns sn and leb len
    fn markup_leb_section(
        &self,
        region: &mut Region<F>,
        wasm_bytecode: &WasmBytecode,
        leb_bytes_offset: usize,
        assign_type: AssignType,
    ) -> (u64, usize) {
        let is_signed = false;
        let (sn, last_byte_offset) = leb128_compute_sn(wasm_bytecode.bytes.as_slice(), is_signed, leb_bytes_offset).unwrap();
        let mut sn_recovered_at_pos = 0;
        let last_byte_rel_offset = last_byte_offset - leb_bytes_offset;
        for byte_rel_offset in 0..=last_byte_rel_offset {
            let offset = leb_bytes_offset + byte_rel_offset;
            sn_recovered_at_pos = leb128_compute_sn_recovered_at_position(
                sn_recovered_at_pos,
                is_signed,
                byte_rel_offset,
                last_byte_rel_offset,
                wasm_bytecode.bytes[offset],
            );
            self.assign(
                region,
                wasm_bytecode,
                offset,
                assign_type,
                1,
                Some(LebParams{
                    is_signed,
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
    pub fn assign_auto(
        &self,
        region: &mut Region<F>,
        wasm_bytecode: &WasmBytecode,
        offset_start: usize,
    ) -> Result<usize, Error> {
        let mut offset = offset_start;

        let (items_count, items_count_leb_len) = self.markup_leb_section(
            region,
            wasm_bytecode,
            offset,
            AssignType::IsItemsCount,
        );
        offset += items_count_leb_len;

        for _item_index in 0..items_count {
            let (export_name_len, export_name_len_leb_len) = self.markup_leb_section(
                region,
                wasm_bytecode,
                offset,
                AssignType::IsExportNameLen,
            );
            offset += export_name_len_leb_len;

            let export_name_new_offset = self.markup_name_section(
                region,
                wasm_bytecode,
                offset,
                AssignType::IsExportName,
                export_name_len as usize,
            );
            offset = export_name_new_offset;

            let exportdesc_type = wasm_bytecode.bytes.as_slice()[offset];
            self.assign(
                region,
                wasm_bytecode,
                offset,
                AssignType::IsExportdescType,
                1,
                None,
            );
            offset += 1;

            let (exportdesc_val, exportdesc_val_leb_len) = self.markup_leb_section(
                region,
                wasm_bytecode,
                offset,
                AssignType::IsExportdescVal,
            );
            offset += exportdesc_val_leb_len;
        }

        Ok(offset)
    }
}