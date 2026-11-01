//! The small JavaScript `Model` surface that the React host uses.
//!
//! IronCalc owns workbook editing and the selected view. This wrapper keeps
//! the engine borrow in Rust so the renderer can paint the same model.

use ironcalc_base::UserModel;
use wasm_bindgen::prelude::*;

fn to_js_error(error: String) -> JsError {
    JsError::new(&error)
}

#[wasm_bindgen]
pub struct Model {
    model: UserModel<'static>,
}

#[wasm_bindgen]
impl Model {
    #[wasm_bindgen(constructor)]
    pub fn new(
        name: &str,
        locale: &str,
        timezone: &str,
        language_id: &str,
    ) -> Result<Model, JsError> {
        // Start from static defaults, then copy the host strings into the
        // owned workbook. This keeps the engine borrow static without leaking
        // the JavaScript inputs.
        let mut model = UserModel::new_empty("", "en", "UTC", "en").map_err(to_js_error)?;
        model.set_name(name);
        model.set_timezone(timezone).map_err(to_js_error)?;
        model.set_locale(locale).map_err(to_js_error)?;
        model.set_language(language_id).map_err(to_js_error)?;
        Ok(Self { model })
    }

    /// Load an XLSX workbook from bytes.
    #[wasm_bindgen(js_name = "fromXlsx")]
    pub fn from_xlsx(
        bytes: &[u8],
        name: &str,
        locale: &str,
        timezone: &str,
        language_id: &str,
    ) -> Result<Model, JsError> {
        let workbook = ironcalc::import::load_from_xlsx_bytes(bytes, name, locale, timezone)
            .map_err(|error| JsError::new(&error.to_string()))?;
        let calc_model =
            ironcalc_base::Model::from_workbook(workbook, "en").map_err(to_js_error)?;
        let mut model: UserModel<'static> = UserModel::from_model(calc_model);
        model.set_name(name);
        model.set_timezone(timezone).map_err(to_js_error)?;
        model.set_locale(locale).map_err(to_js_error)?;
        model.set_language(language_id).map_err(to_js_error)?;
        Ok(Self { model })
    }

    #[wasm_bindgen(js_name = "getName")]
    pub fn get_name(&self) -> String {
        self.model.get_name()
    }

    #[wasm_bindgen(js_name = "getSelectedSheet")]
    pub fn get_selected_sheet(&self) -> u32 {
        self.model.get_selected_sheet()
    }

    #[wasm_bindgen(js_name = "getSelectedView")]
    pub fn get_selected_view(&self) -> Result<JsValue, JsError> {
        serde_wasm_bindgen::to_value(&self.model.get_selected_view())
            .map_err(|error| to_js_error(error.to_string()))
    }

    #[wasm_bindgen(js_name = "getWorksheetsProperties")]
    pub fn get_worksheets_properties(&self) -> Result<JsValue, JsError> {
        serde_wasm_bindgen::to_value(&self.model.get_worksheets_properties())
            .map_err(|error| to_js_error(error.to_string()))
    }

    #[wasm_bindgen(js_name = "setSelectedSheet")]
    pub fn set_selected_sheet(&mut self, sheet: u32) -> Result<(), JsError> {
        self.model.set_selected_sheet(sheet).map_err(to_js_error)
    }

    #[wasm_bindgen(js_name = "setSelectedCell")]
    pub fn set_selected_cell(&mut self, row: i32, column: i32) -> Result<(), JsError> {
        self.model
            .set_selected_cell(row, column)
            .map_err(to_js_error)
    }

    #[wasm_bindgen(js_name = "setSelectedRange")]
    pub fn set_selected_range(
        &mut self,
        start_row: i32,
        start_column: i32,
        end_row: i32,
        end_column: i32,
    ) -> Result<(), JsError> {
        self.model
            .set_selected_range(start_row, start_column, end_row, end_column)
            .map_err(to_js_error)
    }

    #[wasm_bindgen(js_name = "setUserInput")]
    pub fn set_user_input(
        &mut self,
        sheet: u32,
        row: i32,
        column: i32,
        value: &str,
    ) -> Result<(), JsError> {
        self.model
            .set_user_input(sheet, row, column, value)
            .map_err(to_js_error)
    }

    #[wasm_bindgen(js_name = "newSheet")]
    pub fn new_sheet(&mut self) -> Result<(), JsError> {
        self.model.new_sheet().map_err(to_js_error)
    }
}

impl Model {
    pub fn user_model(&self) -> &UserModel<'static> {
        &self.model
    }

    pub fn user_model_mut(&mut self) -> &mut UserModel<'static> {
        &mut self.model
    }
}
