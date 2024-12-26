use crate::commodity::CommodityPolling;

pub struct Poll;
impl Poll {
    fn new() -> Self {
        Self {}
    }

    async fn poll_commodity() {
        let commodities = CommodityPolling::new();

        let list = commodities.list().await
            .map_err(|err| {println!("Error polling commodities: {:?}", err)})
            .unwrap();
    }
}
