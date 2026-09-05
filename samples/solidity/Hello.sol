// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract Greeter {
    mapping(address => string) public greetings;

    event GreetingSet(address indexed sender, string greeting);

    modifier notEmpty(string memory greeting) {
        assert(bytes(greeting).length > 0);
        _;
    }

    function setGreeting(string memory greeting) public notEmpty(greeting) {
        greetings[msg.sender] = greeting;
        emit GreetingSet(msg.sender, greeting);
    }
}
