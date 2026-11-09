use super::files::{COPY_EFFECT, DROP_EFFECT, global};
use anyhow::{Context, Result};
use std::mem::ManuallyDrop;
use windows::Win32::Foundation::GlobalFree;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, CoCreateInstance, DVASPECT_CONTENT, FORMATETC, IDataObject, STGMEDIUM, STGMEDIUM_0,
    TYMED_HGLOBAL,
};
use windows::Win32::System::DataExchange::RegisterClipboardFormatW;
use windows::Win32::System::Ole::{DROPEFFECT_COPY, DROPEFFECT_LINK, DROPEFFECT_MOVE};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    BHID_DataObject, CLSID_DragDropHelper, DSH_ALLOWDROPDESCRIPTIONTEXT, IDragSourceHelper2, ILFree, IShellItemArray,
    SHCreateShellItemArrayFromIDLists, SHDoDragDrop, SHParseDisplayName,
};
use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXDRAG, SM_CYDRAG};
use windows::core::HSTRING;

pub(crate) fn drag_distance() -> (i32, i32) {
    unsafe { (GetSystemMetrics(SM_CXDRAG), GetSystemMetrics(SM_CYDRAG)) }
}

struct Items(Vec<*mut ITEMIDLIST>);

impl Drop for Items {
    fn drop(&mut self) {
        for &item in &self.0 {
            unsafe { ILFree(Some(item)) };
        }
    }
}

pub(crate) fn drag(paths: &[String]) -> Result<()> {
    let mut items = Items(Vec::with_capacity(paths.len()));
    for path in paths {
        let mut item = std::ptr::null_mut();
        unsafe { SHParseDisplayName(&HSTRING::from(path.as_str()), None, &mut item, 0, None) }
            .with_context(|| format!("finding {path} in the shell"))?;
        items.0.push(item);
    }
    let list: Vec<*const ITEMIDLIST> = items.0.iter().map(|&item| item.cast_const()).collect();
    let array: IShellItemArray =
        unsafe { SHCreateShellItemArrayFromIDLists(&list) }.context("collecting the dragged items")?;
    let data: IDataObject =
        unsafe { array.BindToHandler(None, &BHID_DataObject) }.context("packing the items for a drag")?;
    prefer_copy(&data)?;
    if let Err(error) = picture(&data) {
        log::warn!("{error:#}");
    }
    unsafe { SHDoDragDrop(None, &data, None, DROPEFFECT_COPY | DROPEFFECT_MOVE | DROPEFFECT_LINK) }
        .context("dragging the items")?;
    Ok(())
}

fn prefer_copy(data: &IDataObject) -> Result<()> {
    let memory = global(size_of::<u32>(), |memory| unsafe { memory.cast::<u32>().write_unaligned(COPY_EFFECT) })?;
    let format = FORMATETC {
        cfFormat: unsafe { RegisterClipboardFormatW(&HSTRING::from(DROP_EFFECT)) } as u16,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as _,
    };
    let medium = STGMEDIUM {
        tymed: TYMED_HGLOBAL.0 as _,
        u: STGMEDIUM_0 { hGlobal: memory },
        pUnkForRelease: ManuallyDrop::new(None),
    };
    if let Err(error) = unsafe { data.SetData(&format, &medium, true) } {
        let _ = unsafe { GlobalFree(Some(memory)) };
        return Err(error).context("marking the drag as a copy");
    }
    Ok(())
}

fn picture(data: &IDataObject) -> Result<()> {
    let helper: IDragSourceHelper2 =
        unsafe { CoCreateInstance(&CLSID_DragDropHelper, None, CLSCTX_INPROC_SERVER) }.context("drag image helper")?;
    unsafe {
        helper.SetFlags(DSH_ALLOWDROPDESCRIPTIONTEXT.0 as _).context("allowing drop descriptions")?;
        helper.InitializeFromWindow(None, None, data).context("making the drag image")
    }
}
