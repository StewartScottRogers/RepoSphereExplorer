// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

/// @title A milestone escrow between a payer and a payee
/// @notice Funds are locked, released per milestone, and refundable after
///         a deadline if the work never lands. Written to show the shapes
///         a Solidity preview should surface: interfaces, libraries,
///         abstract contracts, inheritance, events, errors and modifiers.

interface IEscrow {
    function deposit() external payable;
    function release(uint256 milestoneId) external;
    function refund() external;
    function balanceOf(address account) external view returns (uint256);
}

library SafeMath96 {
    error Overflow(uint256 value);

    function toUint96(uint256 value) internal pure returns (uint96) {
        if (value > type(uint96).max) {
            revert Overflow(value);
        }
        return uint96(value);
    }

    function percentOf(uint256 value, uint8 percent) internal pure returns (uint256) {
        return (value * percent) / 100;
    }
}

abstract contract Ownable {
    address public owner;

    event OwnershipTransferred(address indexed from, address indexed to);

    error NotOwner(address caller);

    constructor(address initialOwner) {
        owner = initialOwner;
        emit OwnershipTransferred(address(0), initialOwner);
    }

    modifier onlyOwner() {
        if (msg.sender != owner) {
            revert NotOwner(msg.sender);
        }
        _;
    }

    function transferOwnership(address next) external onlyOwner {
        emit OwnershipTransferred(owner, next);
        owner = next;
    }
}

contract MilestoneEscrow is IEscrow, Ownable {
    using SafeMath96 for uint256;

    enum Status {
        Funding,
        Active,
        Complete,
        Refunded
    }

    struct Milestone {
        string description;
        uint96 amount;
        bool released;
    }

    address public immutable payee;
    uint256 public immutable deadline;
    Status public status;

    Milestone[] private milestones;
    mapping(address => uint256) private deposits;

    event Deposited(address indexed from, uint256 amount);
    event Released(uint256 indexed milestoneId, uint256 amount);
    event Refunded(address indexed to, uint256 amount);

    error WrongStatus(Status expected, Status actual);
    error UnknownMilestone(uint256 milestoneId);
    error AlreadyReleased(uint256 milestoneId);
    error TooEarly(uint256 deadlineAt, uint256 nowAt);
    error NothingToRefund(address caller);

    constructor(address payee_, uint256 durationSeconds, address owner_) Ownable(owner_) {
        payee = payee_;
        deadline = block.timestamp + durationSeconds;
        status = Status.Funding;
    }

    modifier inStatus(Status expected) {
        if (status != expected) {
            revert WrongStatus(expected, status);
        }
        _;
    }

    function addMilestone(string calldata description, uint256 amount)
        external
        onlyOwner
        inStatus(Status.Funding)
        returns (uint256 milestoneId)
    {
        milestones.push(Milestone({
            description: description,
            amount: amount.toUint96(),
            released: false
        }));
        return milestones.length - 1;
    }

    function deposit() external payable override inStatus(Status.Funding) {
        deposits[msg.sender] += msg.value;
        emit Deposited(msg.sender, msg.value);

        if (address(this).balance >= totalCommitted()) {
            status = Status.Active;
        }
    }

    function release(uint256 milestoneId) external override onlyOwner inStatus(Status.Active) {
        if (milestoneId >= milestones.length) {
            revert UnknownMilestone(milestoneId);
        }

        Milestone storage milestone = milestones[milestoneId];
        if (milestone.released) {
            revert AlreadyReleased(milestoneId);
        }

        milestone.released = true;
        emit Released(milestoneId, milestone.amount);

        (bool sent, ) = payable(payee).call{value: milestone.amount}("");
        require(sent, "transfer failed");

        if (allReleased()) {
            status = Status.Complete;
        }
    }

    function refund() external override {
        if (block.timestamp < deadline) {
            revert TooEarly(deadline, block.timestamp);
        }

        uint256 owed = deposits[msg.sender];
        if (owed == 0) {
            revert NothingToRefund(msg.sender);
        }

        deposits[msg.sender] = 0;
        status = Status.Refunded;
        emit Refunded(msg.sender, owed);

        (bool sent, ) = payable(msg.sender).call{value: owed}("");
        require(sent, "refund failed");
    }

    function balanceOf(address account) external view override returns (uint256) {
        return deposits[account];
    }

    function milestoneCount() external view returns (uint256) {
        return milestones.length;
    }

    function totalCommitted() public view returns (uint256 total) {
        for (uint256 i = 0; i < milestones.length; i++) {
            total += milestones[i].amount;
        }
    }

    function allReleased() public view returns (bool) {
        for (uint256 i = 0; i < milestones.length; i++) {
            if (!milestones[i].released) {
                return false;
            }
        }
        return milestones.length > 0;
    }
}
