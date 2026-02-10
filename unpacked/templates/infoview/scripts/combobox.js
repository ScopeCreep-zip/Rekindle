/********************************************************************
* HTML ComboBox
* Copyright (C)2005 CS Wagner.
* This provides ComboBox functionality for HTML.
*********************************************************************
* HOWTO:
* You may use this on as many select elements as you want.
* For each select element, add the following three lines:
*   onfocus='javascript:return combo_focus(this);'
*   onchange='javascript:return combo_focus(this);'
*   onkeydown='javascript:return combo_keydown(this,event);'
*   onkeypress='javascript:return combo_keypress(this,event);'
*   onkeyup='javascript:return combo_keyup(this,event);'
* Also, make sure the first option is empty, as in:
*   <option value=''></option>
*********************************************************************
* HOW IT WORKS:
* This attempts to capture key presses and alter the value of the
* first option in the select list.  Doing so can be very complicated
* because every web browser seems to process key press events in a
* different way.
* Others have tried this and have hit common problems.  The ones that
* I know of, I have tried to handle:
* Backspace Problem:
*   When you hit backspace, you go to the previous web page.
*   I fixed this by cancelling the default backspace function.
* Auto-Selection of Options:
*   When you type the first letter of an option, the option is
*   selected automatically.
*   I fixed this by cancelling the default auto-select function.
* Edit of Existing Options:
*   Can user edit an option in the list?
*   When you edit while an option is selected, the edits replace the
*   first (editable) item in the list.
*********************************************************************
* COMPATABILITY:
* If the user does not have Javascript, this will not work.
* However, they will easily use the Select field with the values in
* the field.
* I have tried very hard to use Javascript that is common to all web
* browsers, so it will function to some degree for everyone.
*
* I have tested this with:
* Firefox 1.0.7: There is a bug in Firefox. When you set a select
*   box's index, it doesn't always update.  Also, when you cancel the
*   auto-select, it still auto-selects sometimes.
* Konqueror 3.5: This works fine.
* Internet Exlporer 6.0: It is functional, but many keys do not work.
*   I have added work-arounds for all letter/number keys.
* Opera: I have tested it in the past and hitting backspace is always
*   treated as clicking the back button.
*********************************************************************/

var combo_word;	// The user-editable word
var combo_keypress_called;	// A fix for Konqueror/Safari

// When select is focused, set the editable word.
function combo_focus(element)
{
	combo_word = "";
	if(element.selectedIndex >= 0)
		combo_word = element.options[element.selectedIndex].text;
	return true;
}

// Same function as focus, but here to make the HTML less confusing.
function combo_change(element)
{
	return combo_focus(element);
}

// Return false to try and block built-in keydown functions.
function combo_keydown(element, event)
{
	var c=0;
	if(event.keyCode) c = event.keyCode;
	else if(event.charCode) c = event.charCode;
	else if (event.which) c = event.which;
	// Don't block the tab key
	if(c == 9) return true;

	// We have not called keypress yet.
	combo_keypress_called = false;
	return false;
}

// The real work.  Update the editable word and the select box.
function combo_keypress(element, event)
{
	alert("keypress");
	
	// Do not call this again on keyup.
	combo_keypress_called = true;

	// Get the current key pressed.
	var c=0;
	if(event.keyCode) c = event.keyCode;
	else if(event.charCode) c = event.charCode;
	else if (event.which) c = event.which;
	// Don't block the tab key
	if(c == 9) return true;

	// Konqueror sends all alpha-characters in as upper case.
	// This converts them to lower case if shift is not pressed.
	if(!event.shiftKey)
	{
		if(c>=65 && c<=90) c+= 32;
	}
	// IE doesn't send the numbers shifted.
	else
	{
		if(c == 48) c = 41;
		else if(c == 49) c = 33;
		else if(c == 50) c = 64;
		else if(c == 51) c = 35;
		else if(c == 52) c = 36;
		else if(c == 53) c = 37;
		else if(c == 54) c = 94;
		else if(c == 55) c = 38;
		else if(c == 56) c = 42;
		else if(c == 57) c = 40;
	}

	if(c>=32 && c<=126)	// Printable characters
	{
		combo_word += String.fromCharCode(c);
		element.options[0].value = combo_word;
		element.options[0].text = combo_word;
		var combo_wlc = combo_word.toLowerCase();
		var combo_select = 0;
		for(i=1; i<element.options.length; i++)
		{
			combo_sel = element.options[i].text.toLowerCase();
			if(combo_wlc.length <= combo_sel.length)
			{
				combo_sel = combo_sel.substring(0, combo_wlc.length);
				if(combo_wlc == combo_sel)
					combo_select = i;
			}
		}
		element.selectedIndex = combo_select;
	}
	else if((c == 8 || c == 4099) && combo_word.length>0)	// Backspace
	{
		combo_word = combo_word.substring(0, combo_word.length-1);
		element.options[0].value = combo_word;
		element.options[0].text = combo_word;
		element.selectedIndex = 0;
	}

	return false;
}

// Return false to try and block built-in keyup functions (like auto-select).
function combo_keyup(element, event)
{
	var c=0;
	if(event.keyCode) c = event.keyCode;
	else if(event.charCode) c = event.charCode;
	else if (event.which) c = event.which;
	// Don't block the tab key
	if(c == 9) return true;

	// If keypress was not called, call it now.
	if(!combo_keypress_called)
		return combo_keypress(element, event);
	return false;
}