use super::global::Delivery;

pub(crate) fn remote_delivery(solicited: bool, away: bool) -> Delivery {
    if solicited {
        Delivery::Explicit
    } else {
        Delivery::Presence { away }
    }
}

#[cfg(test)]
mod tests;
