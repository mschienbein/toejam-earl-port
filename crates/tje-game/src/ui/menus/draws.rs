use crate::{
    Context, Result, call, call_at,
    call_frame::{CallSite, FrameLayout, LocalFrame},
    ids,
};
use tje_mem::{Address, MenuItem};

/// G/src/ui/main_menu.c:283 (`MenuDrawTextItems`).
pub fn menu_draw_text_items(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let mut item = Address::<MenuItem>::new(arguments[0]);
    let count = arguments[1] as i16;
    for _ in 0..count {
        let x = u32::from(item.x().read(context));
        let y = u32::from(item.y().read(context));
        context.write_vdp_command(
            0x50000003_u32
                .wrapping_add(y.wrapping_shl(23))
                .wrapping_add(x.wrapping_shl(17)),
        );
        let text = item.text().read(context).address();
        call(context, ids::TEXT_WRITE_STRING_TILES, &[text])?;
        item = item.offset(1);
    }
    Ok(None)
}

/// G/src/ui/inventory.c:706 (`TextWriteDecimalTiles`).
/// c_inventory.s:2021-2082 (REV 02), 2016-2077 (REV 00): LINK -12,
/// MOVEM D2-D6 and the actual A6-12 array passed at the +0x8a return site.
/// Negating -32768 deliberately narrows back to a negative short.
pub fn text_write_decimal_tiles(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let frame = LocalFrame::enter(context, FrameLayout::link(12, 0x7c, 0))?;
    let text = frame.local::<i8>(-12);
    let mut value = arguments[0] as i16;
    let mut count = 0_i16;
    let mut started = false;
    if value < 0 {
        text.offset(i32::from(count)).write(context, b'-' as i8);
        count = count.wrapping_add(1);
        value = value.wrapping_neg();
    }
    let mut divisor = 10000_i16;
    for position in 0..5 {
        let digit = (i32::from(value) / i32::from(divisor)) as i16;
        frame.local::<i16>(-2).write(context, digit);
        if digit != 0 || started {
            value = (i32::from(value) % i32::from(divisor)) as i16;
            text.offset(i32::from(count))
                .write(context, (i32::from(digit) + i32::from(b'0')) as i8);
            count = count.wrapping_add(1);
            started = true;
        }
        divisor = (i32::from(divisor) / 10) as i16;
        if position == 3 {
            started = true;
        }
    }
    text.offset(i32::from(count)).write(context, 0);
    let site = CallSite::relative(ids::TEXT_WRITE_DECIMAL_TILES, context.revision(), 0x8a);
    call_at(
        context,
        ids::TEXT_WRITE_STRING_TILES,
        &[text.address()],
        site,
    )?;
    frame.leave(context)?;
    Ok(None)
}
