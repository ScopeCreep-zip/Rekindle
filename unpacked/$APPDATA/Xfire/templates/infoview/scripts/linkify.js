function linkify(str)
{
	// looks through a string for http:// (or www.) and puts the url
	// in an anchor tag
	var retval = '';

	while (str.length > 0)
	{
		var index = str.indexOf('http://');
		if (index < 0)
			index = str.indexOf('https://');
		if (index < 0)
			index = str.indexOf('ftp://');
		if (index < 0)
			index = str.indexOf('xfire:');
		if (index < 0)
			index = str.indexOf('www.');

		if (index < 0)
		{
			retval += str;
			break;
		}
		else
		{
			var linktext = str.substring(0,index);
			str = str.substring(index);
			index = str.indexOf(' ');
			var url;
			if (index < 0)
			{
				// url ends the status
				url = str;
				str = '';
			}
			else
			{
				url = str.substring(0,index);
				str = str.substring(index);
			}
			var post_prepend = '';
			if (linktext.length == 0)
				linktext = url;
			else
			{
				// take out a trailing space
				if (linktext.charAt(linktext.length-1) == ' ')
				{
					linktext = linktext.substring(0,linktext.length-1);
					post_prepend = ' ';
				}
			}
			if (url.indexOf('www.') == 0)
				url = 'http://' + url;
			retval += "<a title=\"" + url + "\" href=\"";
			retval += url + "\" target=\"_blank\">" + linktext + "</a>";
			// only put on the space (if needed) if not at the end
			if (str.length > 0)
				retval += post_prepend;
		}
	}

	return retval;
}
