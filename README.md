# slot-machine

A repo containing the main CLI and code used to interact with Chia blockchain dApps using the slot primitive and the action layer: CATalog, XCHandles, and the Reward Distributor.

## CATalog

A decentralized, shared CAT registration system for the Chia blockchain. It allows CAT creators to register new CATs and add useful information such as name and image URL. Verifiers can then create verification coins to attest that the data is not misleading, as well as assign custom metadata to the CAT that their website/apps later use.


## XCHandles

A decentralized naming system/address book for the Chia blockchain. XCHandles anyone to register a name and associate it to an address. Users receive an NFT that they can fully customize, with the ability to also add fields such as profile pictures.

## Reward Distributor

A transaction-efficient distribution system. Anyone can commit rewards for the current or future distribution periods, which usually last one week. The reward amount is efficiently distributed among eligible recipients. Some reward distributors act as permissioned farms, where a manager controls the list of currently active reward addresses, as well as their weight. The other mode allows anyone to stake any NFT minted by a given DID to start earning rewards right away - users may unstake an NFT at any time.

## Learn More

Want to learn more? Here are some resources that might help (in recommended order):
 * [Post: The Problem of Uniqueness](https://blog.fireacademy.io/p/uniqueness-on-chain)
 * [Post: Solving the Problem of Uniqueness](https://blog.fireacademy.io/p/solving-the-problem-of-uniqueness)
 * [Post: Announcing CATalog and XCHandles](https://blog.fireacademy.io/p/announcing-catalog-and-xchandles)
 * [Presentation](https://pitch.com/v/uniqueness-fjrbf7)
 * [DIP-0002: DIG Reward Distributor](https://github.com/DIG-Network/DIPS/blob/main/DIPs/dip-0002.md)
 * [Paper on the core principle behind the Reward Distributor](https://uploads-ssl.webflow.com/5ad71ffeb79acc67c8bcdaba/5ad8d1193a40977462982470_scalable-reward-distribution-paper.pdf)
 * [CATalog Docs: Technical Manual](https://docs.catalog.cat/)
 * [XCHandles Docs: Technical Manual](https://docs.xchandles.com/)

## License

This repo is licensed under the MIT license - see the [LICENSE](LICENSE) for more details.