// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

import {ERC721} from "solady/tokens/ERC721.sol";

/// A test collection for the TRON (Nile) demo: anyone may mint. Testnet only.
contract MarviiDemoNFT is ERC721 {
    uint256 public minted;

    function name() public pure override returns (string memory) {
        return "Marvii Demo";
    }

    function symbol() public pure override returns (string memory) {
        return "MVDEMO";
    }

    function tokenURI(uint256) public pure override returns (string memory) {
        return "ipfs://marvii-demo-agent";
    }

    function mint(address to) external returns (uint256 id) {
        id = ++minted;
        _mint(to, id);
    }
}
