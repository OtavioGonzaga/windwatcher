local id = ARGV[1]
local k = key(id)
check_type(k,'hash')
if not fenced(k,ARGV[2],ARGV[3]) then return 0 end
local attempts, maximum = counters(k)
member(k,id)
remove_due(k,id,'leased')
clear_lease(k)
redis.call('HSET',k,'last_error_code',ARGV[4],'last_error_message',ARGV[5],'available_at',ARGV[6])
if attempts < maximum then
    move(k,id,'leased','pending')
    redis.call('ZADD',due('pending'),0,ARGV[6] .. ':' .. id)
    return 1
end
move(k,id,'leased','failed')
redis.call('HSET',k,'failed_at',ARGV[3])
return 2
